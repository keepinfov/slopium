//! Human-readable MIR rendering.
//!
//! `--emit mir` produces JSON, which is fine for machines and unreadable when
//! you are trying to see why a pass did something. This renders the same module
//! in a form meant to be read, with each statement annotated by the source line
//! it came from.
//!
//! Output is deterministic: everything is driven off ordered vectors.

use crate::mir::{
    BasicBlock, BinaryOp, Instruction, MirEnum, MirFunction, MirModule, MirStruct, Terminator,
};
use std::fmt::Write;

/// Column at which the `// line:column` provenance comment starts.
const COMMENT_COLUMN: usize = 48;

pub fn render_module(module: &MirModule) -> String {
    let mut out = String::new();
    for structure in &module.structs {
        render_struct(&mut out, structure);
    }
    for enumeration in &module.enums {
        render_enum(&mut out, enumeration);
    }
    for function in &module.functions {
        render_function_into(&mut out, function, None);
    }
    for test in &module.tests {
        render_function_into(&mut out, &test.function, Some(&test.name));
    }
    out
}

pub fn render_function(function: &MirFunction) -> String {
    let mut out = String::new();
    render_function_into(&mut out, function, None);
    out
}

fn not_emitted(emit: bool) -> &'static str {
    if emit {
        ""
    } else {
        "  // not emitted"
    }
}

fn render_struct(out: &mut String, structure: &MirStruct) {
    let _ = writeln!(
        out,
        "struct {} {{{}",
        structure.name,
        not_emitted(structure.emit)
    );
    for (name, ty) in &structure.fields {
        let _ = writeln!(out, "    {name}: {ty},");
    }
    let _ = writeln!(out, "}}\n");
}

fn render_enum(out: &mut String, enumeration: &MirEnum) {
    let _ = writeln!(
        out,
        "enum {} {{{}",
        enumeration.name,
        not_emitted(enumeration.emit)
    );
    for variant in &enumeration.variants {
        let fields = variant
            .fields
            .iter()
            .map(|(name, ty)| format!("{name}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "    {} = {} ({fields}),", variant.name, variant.tag);
    }
    let _ = writeln!(out, "}}\n");
}

fn render_function_into(out: &mut String, function: &MirFunction, test_name: Option<&str>) {
    if let Some(name) = test_name {
        let _ = writeln!(out, "// test {name:?}");
    }
    let params = function
        .params
        .iter()
        .map(|param| {
            let ty = function
                .locals
                .get(*param)
                .map_or_else(|| "?".to_owned(), |local| local.ty.to_string());
            format!("_{param}: {ty}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(
        out,
        "fn {}({params}) -> {} {{{}",
        function.name,
        function.return_type,
        not_emitted(function.emit)
    );

    for (index, local) in function.locals.iter().enumerate() {
        if local.is_param {
            continue;
        }
        match &local.name {
            Some(name) => {
                let _ = writeln!(out, "    let _{index}: {};  // {name}", local.ty);
            }
            None => {
                let _ = writeln!(out, "    let _{index}: {};", local.ty);
            }
        }
    }
    if function.locals.iter().any(|local| !local.is_param) {
        out.push('\n');
    }

    for (id, block) in function.blocks.iter().enumerate() {
        let entry = if id == function.entry {
            "  // entry"
        } else {
            ""
        };
        let _ = writeln!(out, "  bb{id}:{entry}");
        render_block(out, block);
        if id + 1 != function.blocks.len() {
            out.push('\n');
        }
    }
    let _ = writeln!(out, "}}\n");
}

fn render_block(out: &mut String, block: &BasicBlock) {
    for statement in &block.statements {
        let text = render_instruction(&statement.instruction);
        write_with_location(
            out,
            &text,
            statement.span.line,
            statement.span.column,
            statement.span.start != 0 || statement.span.line > 1,
        );
    }
    let terminator = render_terminator(&block.terminator);
    write_with_location(
        out,
        &terminator,
        block.terminator_span.line,
        block.terminator_span.column,
        block.terminator_span.start != 0 || block.terminator_span.line > 1,
    );
}

fn write_with_location(out: &mut String, text: &str, line: usize, column: usize, located: bool) {
    let rendered = format!("    {text}");
    if !located {
        let _ = writeln!(out, "{rendered}");
        return;
    }
    let padding = COMMENT_COLUMN.saturating_sub(rendered.chars().count());
    let _ = writeln!(out, "{rendered}{:padding$}// {line}:{column}", "");
}

fn locals(ids: &[usize]) -> String {
    ids.iter()
        .map(|id| format!("_{id}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn operator(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Less => "<",
        BinaryOp::Greater => ">",
        BinaryOp::Equal => "==",
    }
}

fn render_instruction(instruction: &Instruction) -> String {
    match instruction {
        Instruction::ConstInt { dst, value } => format!("_{dst} = const {value};"),
        Instruction::ConstFloat { dst, bits } => {
            format!("_{dst} = const {};", f64::from_bits(*bits))
        }
        Instruction::ConstBool { dst, value } => format!("_{dst} = const {value};"),
        Instruction::StringNew { dst, value } => format!("_{dst} = const {value:?};"),
        Instruction::Assign { dst, src } => format!("_{dst} = _{src};"),
        Instruction::AddressOf { dst, src } => format!("_{dst} = &_{src};"),
        Instruction::Binary {
            dst,
            op,
            lhs,
            rhs,
            ty,
        } => format!("_{dst} = _{lhs} {} _{rhs}: {ty};", operator(*op)),
        Instruction::Call {
            dst,
            callee,
            args,
            result,
            ..
        } => format!("_{dst} = {callee}({}) -> {result};", locals(args)),
        Instruction::Drop { local, ty } => format!("drop(_{local}: {ty});"),
        Instruction::StructNew { dst, name, fields } => {
            format!("_{dst} = {name} {{ {} }};", locals(fields))
        }
        Instruction::FieldLoad { dst, base, index } => format!("_{dst} = _{base}.{index};"),
        Instruction::EnumNew {
            dst,
            enum_name,
            tag,
            fields,
        } => format!("_{dst} = {enum_name}#{tag}({});", locals(fields)),
        Instruction::EnumTag { dst, base } => format!("_{dst} = tag(_{base});"),
        Instruction::EnumFieldLoad { dst, base, index } => {
            format!("_{dst} = payload(_{base}, {index});")
        }
        Instruction::Free { local } => format!("free(_{local});"),
    }
}

fn render_terminator(terminator: &Terminator) -> String {
    match terminator {
        Terminator::Return(Some(local)) => format!("return _{local};"),
        Terminator::Return(None) => "return;".to_owned(),
        Terminator::Goto(target) => format!("goto -> bb{target};"),
        Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => format!("branch _{condition} -> [true: bb{then_block}, false: bb{else_block}];"),
        Terminator::Unreachable => "unreachable;".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::render_module;
    use crate::{compile_to_mir, CompileOptions};

    #[test]
    fn renders_a_branch_with_source_locations() {
        let source = "(fn main () -> i32 (if true 1 0))";
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let text = render_module(&mir);

        assert!(text.contains("fn main() -> i32 {"), "{text}");
        assert!(text.contains("  bb0:  // entry"), "{text}");
        assert!(
            text.contains("branch _0 -> [true: bb1, false: bb2];"),
            "{text}"
        );
        assert!(text.contains("_0 = const true;"), "{text}");
        assert!(text.contains("goto -> bb3;"), "{text}");
        assert!(text.contains("return _1;"), "{text}");
        // Provenance comments must be present.
        assert!(text.contains("// 1:"), "{text}");
    }

    #[test]
    fn renders_aggregates_drops_and_calls() {
        let source = r#"
            (struct Pair ((left String) (right String)))
            (enum Shape Empty (Sized ((size i64))))
            (fn note ((text String)) -> unit ())
            (fn main () -> i32
              (let pair (Pair :left "l" :right "r"))
              (let shape (Shape:Sized 3))
              (note "hi")
              0)
        "#;
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let text = render_module(&mir);

        assert!(text.contains("struct Pair {"), "{text}");
        assert!(text.contains("    left: String,"), "{text}");
        assert!(text.contains("enum Shape {"), "{text}");
        assert!(text.contains("    Sized = 1 (size: i64),"), "{text}");
        assert!(text.contains("= Pair {"), "{text}");
        assert!(text.contains("drop(_"), "{text}");
        assert!(text.contains("note("), "{text}");
    }

    #[test]
    fn rendering_is_stable_across_runs() {
        let source = r#"
            (fn take ((s String)) -> i32 0)
            (fn main () -> i32
              (let a "aaa")
              (let b "bbb")
              (if true (do (take a) (take b) 0) 0))
        "#;
        let first =
            render_module(&compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap());
        for _ in 0..8 {
            let again = render_module(
                &compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap(),
            );
            assert_eq!(first, again, "MIR rendering must be deterministic");
        }
    }
}

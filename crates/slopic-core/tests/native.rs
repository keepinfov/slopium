use slopic_core::codegen::{emit_assembly, CodegenOptions, SUPPORTED_TARGET};
use slopic_core::{
    compile, compile_to_mir, CompileOptions, CompileRequest, DependencySource, EmitKind,
    LanguageItems, RUNTIME_SOURCE,
};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEST: AtomicUsize = AtomicUsize::new(0);

fn native_program(source: &str) -> (PathBuf, PathBuf) {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("slopic-native-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    fs::create_dir_all(&directory).unwrap();
    let input = directory.join("main.slp");
    let output = directory.join("program");
    fs::write(&input, source).unwrap();
    compile(&CompileRequest {
        input,
        source_root: None,
        dependencies: Vec::new(),
        toolchain_dependencies: Vec::new(),
        output: Some(output.clone()),
        emit: EmitKind::Executable,
        options: CompileOptions::default(),
        runtime: None,
        cc: "cc".into(),
    })
    .unwrap();
    (directory, output)
}

#[test]
fn compiles_and_runs_recursive_native_program() {
    let (directory, executable) = native_program(
        r#"
        (fn fib ((n i64)) -> i64
          (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
        (fn main () -> i32
          (println (fib 10))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "55\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn compiles_and_runs_multi_file_package() {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory =
        std::env::temp_dir().join(format!("slopic-package-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    let source_root = directory.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let input = source_root.join("main.slp");
    let output = directory.join("program");
    fs::write(
        source_root.join("geometry.slp"),
        "(export distance)\n(fn distance ((n i64)) -> i64 (+ n 1))\n",
    )
    .unwrap();
    fs::write(
        &input,
        "(take geometry (distance :as length))\n\
         (fn main () -> i32\n\
           (println (length 41))\n\
           (println (geometry:distance 9))\n\
           0)\n",
    )
    .unwrap();
    compile(&CompileRequest {
        input,
        source_root: Some(source_root),
        dependencies: Vec::new(),
        toolchain_dependencies: Vec::new(),
        output: Some(output.clone()),
        emit: EmitKind::Executable,
        options: CompileOptions::default(),
        runtime: None,
        cc: "cc".into(),
    })
    .unwrap();
    let process = Command::new(output).output().unwrap();
    assert!(process.status.success());
    assert_eq!(String::from_utf8(process.stdout).unwrap(), "42\n10\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn compiles_path_dependency_under_manifest_alias() {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory =
        std::env::temp_dir().join(format!("slopic-dependency-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    let app = directory.join("app");
    let dependency = directory.join("dependency");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(&dependency).unwrap();
    fs::write(
        dependency.join("ops.slp"),
        "(export double)\n(fn double ((value i64)) -> i64 (* value 2))\n",
    )
    .unwrap();
    let input = app.join("main.slp");
    fs::write(
        &input,
        "(take math:ops double)\n(fn main () -> i32 (println (double 21)) 0)\n",
    )
    .unwrap();
    let executable = directory.join("program");
    compile(&CompileRequest {
        input,
        source_root: Some(app),
        dependencies: vec![DependencySource {
            namespace: "math".into(),
            source_root: dependency,
        }],
        toolchain_dependencies: Vec::new(),
        output: Some(executable.clone()),
        emit: EmitKind::Executable,
        options: CompileOptions::default(),
        runtime: None,
        cc: "cc".into(),
    })
    .unwrap();
    let process = Command::new(executable).output().unwrap();
    assert!(process.status.success());
    assert_eq!(String::from_utf8(process.stdout).unwrap(), "42\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn monomorphizes_parametric_generic_functions() {
    let (directory, executable) = native_program(
        r#"
        (fn identity (T) ((value T)) -> T value)
        (fn main () -> i32
          (println (identity 42))
          (let text (identity "generic"))
          (println (& text))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "42\ngeneric\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn monomorphizes_generic_structs_and_enums() {
    let (directory, executable) = native_program(
        r#"
        (struct Box (T) ((value T)))
        (enum Option (T) None (Some ((value T))))
        (fn wrap (T) ((value T)) -> (Box T)
          (Box :value value))

        (fn main () -> i32
          (let wrapped (wrap 1))
          (let boxed (Box :value 2))
          (println (. boxed value))
          (let option (Option:Some 40))
          (println
            (match option
              ((Option:None) 0)
              ((Option:Some value) value)))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "2\n40\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn try_propagates_configured_result_errors() {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("slopic-try-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    fs::create_dir_all(&directory).unwrap();
    let input = directory.join("main.slp");
    let executable = directory.join("program");
    fs::write(
        &input,
        r#"
        (enum Result (T E)
          (Ok ((value T)))
          (Err ((error E))))

        (fn produce ((ok bool)) -> (Result i64 String)
          (if ok
              (Result:Ok 41)
              (Result:Err "failure")))

        (fn forward ((ok bool)) -> (Result i64 String)
          (let value (try (produce ok)))
          (Result:Ok (+ value 1)))

        (fn show ((result (Result i64 String))) -> i64
          (match result
            ((Result:Ok value) value)
            ((Result:Err error)
              (do (println (& error)) 0))))

        (fn main () -> i32
          (println (show (forward true)))
          (println (show (forward false)))
          0)
        "#,
    )
    .unwrap();
    compile(&CompileRequest {
        input,
        source_root: None,
        dependencies: Vec::new(),
        toolchain_dependencies: Vec::new(),
        output: Some(executable.clone()),
        emit: EmitKind::Executable,
        options: CompileOptions {
            language_items: LanguageItems {
                option: None,
                result: Some("Result".into()),
                result_ok: Some("Result:Ok".into()),
                result_err: Some("Result:Err".into()),
            },
            ..CompileOptions::default()
        },
        runtime: None,
        cc: "cc".into(),
    })
    .unwrap();
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "42\nfailure\n0\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn toolchain_std_supplies_option_result_and_language_items() {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("slopic-std-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    let source_root = directory.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let input = source_root.join("main.slp");
    let executable = directory.join("program");
    fs::write(
        &input,
        r#"
        (take std:result Result)
        (take std:option Option)

        (fn produce () -> (Result i64 String)
          (Result:Ok 42))

        (fn forward () -> (Result i64 String)
          (let value (try (produce)))
          (Result:Ok value))

        (fn main () -> i32
          (let mut values (list 7))
          (println
            (match (pop (&mut values))
              ((Option:Some value) value)
              ((Option:None) -1)))
          (println
            (match (pop (&mut values))
              ((Option:Some value) value)
              ((Option:None) 0)))
          (println
            (match (forward)
              ((Result:Ok value) value)
              ((Result:Err error) 0)))
          0)
        "#,
    )
    .unwrap();
    compile(&CompileRequest {
        input,
        source_root: Some(source_root),
        dependencies: Vec::new(),
        toolchain_dependencies: vec!["std".into()],
        output: Some(executable.clone()),
        emit: EmitKind::Executable,
        options: CompileOptions::default(),
        runtime: None,
        cc: "cc".into(),
    })
    .unwrap();
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "7\n0\n42\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn strings_lists_structs_and_tests_use_the_runtime() {
    let (directory, executable) = native_program(
        r#"
        (struct Point ((x i64) (y i64)))
        (fn main () -> i32
          (let message "native")
          (println (& message))
          (let point (Point :x 20 :y 22))
          (let mut values (list (. point x) (. point y)))
          (do (push (&mut values) 10))
          (println (+ (get (& values) 0) (get (& values) 1)))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "native\n42\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn enum_match_transfers_owned_payload() {
    let (directory, executable) = native_program(
        r#"
        (enum Message Empty (Text ((value String))))
        (fn consume ((message Message)) -> i64
          (match message
            ((Message:Empty) 0)
            ((Message:Text value)
              (do (println (& value)) 42))))
        (fn main () -> i32
          (println (consume (Message:Text "payload")))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "payload\n42\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn reads_integer_and_returns_owned_environment_string() {
    let (directory, executable) = native_program(
        r#"
        (fn main () -> i32
          (let expected (read-i64))
          (let key "SLOPIUM_NATIVE_TEST_FLAG")
          (let flag (env (& key)))
          (if (= expected 1337)
              (do (println (& flag)) 0)
              1))
        "#,
    );
    let mut child = Command::new(executable)
        .env("SLOPIUM_NATIVE_TEST_FLAG", "slopium{runtime_io_works}")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"1337\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "slopium{runtime_io_works}\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn reads_lines_parses_numbers_and_exposes_process_arguments() {
    let (directory, executable) = native_program(
        r#"
        (fn main () -> i32
          (let line (read-line))
          (println (parse-i64 (& line)))
          (println (args-len))
          (let first (arg 0))
          (println (& first))
          0)
        "#,
    );
    let mut child = Command::new(executable)
        .arg("hello")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b" 42\r\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "42\n1\nhello\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn runtime_errors_use_stable_message_and_exit_status() {
    let (directory, executable) = native_program(
        r#"
        (fn main () -> i32
          (let text "not-a-number")
          (println (parse-i64 (& text)))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "slopium runtime error: invalid i64\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn list_owns_and_moves_non_copy_elements() {
    let (directory, executable) = native_program(
        r#"
        (fn main () -> i32
          (let mut values (list "first" "second" "third"))
          (let first (get-ref (& values) 0))
          (println first)
          (let removed (remove (&mut values) 1))
          (println (& removed))
          (let last (remove (&mut values) 1))
          (println (& last))
          (println (len (& values)))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "first\nsecond\nthird\n1\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn clone_is_structural_for_lists_structs_and_enums() {
    let (directory, executable) = native_program(
        r#"
        (struct Payload ((label String) (values (List String))))
        (enum Message
          Empty
          (Data ((payload Payload))))
        (fn main () -> i32
          (let original
            (Message:Data
              (Payload :label "owned" :values (list "one" "two"))))
          (let copied (clone original))
          (println 42)
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "42\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn loops_support_break_continue_and_mutation() {
    let (directory, executable) = native_program(
        r#"
        (fn main () -> i32
          (let mut n 0)
          (while (< n 5)
            (set n (+ n 1))
            (if (= n 2) (continue) ())
            (if (= n 4) (break) ())
            (println n))
          (println n)
          (loop
            (set n (+ n 1))
            (if (= n 6) (break) ()))
          (println n)
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "1\n3\n4\n6\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn arrays_and_borrowed_slices_support_owned_elements() {
    let (directory, executable) = native_program(
        r#"
        (fn main () -> i32
          (let values (array "zero" "one" "two" "three"))
          (let view (slice (& values) 1 3))
          (println (len (& view)))
          (let first (get-ref (& view) 0))
          (println first)
          (let copied (clone values))
          (let last (get-ref (& copied) 3))
          (println last)
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "2\none\nthree\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn nested_enum_and_struct_patterns_destructure_owned_values() {
    let (directory, executable) = native_program(
        r#"
        (struct Point ((x i64) (label String)))
        (enum Inner
          None
          (Value ((number i64))))
        (enum Outer
          Empty
          (Wrap ((inner Inner)))
          (Pointed ((point Point))))
        (fn score ((value Outer)) -> i64
          (match value
            ((Outer:Wrap (Inner:Value number)) number)
            ((Outer:Wrap (Inner:None)) 0)
            ((Outer:Pointed (Point :x x :label label))
              (do (println (& label)) x))
            ((Outer:Empty) -1)
            (_ -2)))
        (fn main () -> i32
          (println (score (Outer:Wrap (Inner:Value 42))))
          (println (score (Outer:Pointed (Point :x 7 :label "point"))))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "42\npoint\n7\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mixed_register_and_stack_arguments_round_trip() {
    let (directory, executable) = native_program(
        r#"
        (fn probe
          ((i0 i64) (f0 f64) (i1 i64) (f1 f64) (i2 i64) (f2 f64)
           (i3 i64) (f3 f64) (i4 i64) (f4 f64) (i5 i64) (f5 f64)
           (i6 i64) (f6 f64) (i7 i64) (f7 f64) (f8 f64))
          -> i64
          (if (= f8 9.0) (+ i6 i7) 0))
        (fn main () -> i32
          (println (probe 1 1.0 2 2.0 3 3.0 4 4.0 5 5.0 6 6.0
                          7 7.0 8 8.0 9.0))
          0)
        "#,
    );
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "15\n");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn c_caller_agrees_with_slopium_stack_parameter_layout() {
    let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("slopic-c-abi-{}-{id}", std::process::id()));
    if directory.exists() {
        fs::remove_dir_all(&directory).unwrap();
    }
    fs::create_dir_all(&directory).unwrap();
    let source = r#"
        (fn abi_probe
          ((i0 i64) (f0 f64) (i1 i64) (f1 f64) (i2 i64) (f2 f64)
           (i3 i64) (f3 f64) (i4 i64) (f4 f64) (i5 i64) (f5 f64)
           (i6 i64) (f6 f64) (i7 i64) (f7 f64) (f8 f64))
          -> i64
          (if (= f8 9.0) (+ i6 i7) 0))
        (fn main () -> i32 0)
    "#;
    let module = compile_to_mir("abi.slp", source, &CompileOptions::default()).unwrap();
    let assembly = emit_assembly(
        "abi.slp",
        &module,
        &CodegenOptions {
            target: SUPPORTED_TARGET.into(),
            test_harness: false,
            emit_entrypoint: false,
        },
    )
    .unwrap();
    let assembly_path = directory.join("abi.s");
    let runtime_path = directory.join("runtime.c");
    let oracle_path = directory.join("oracle.c");
    let executable = directory.join("oracle");
    fs::write(&assembly_path, assembly).unwrap();
    fs::write(&runtime_path, RUNTIME_SOURCE).unwrap();
    fs::write(
        &oracle_path,
        r#"
        #include <stdint.h>
        extern int64_t abi_probe(
            int64_t, double, int64_t, double, int64_t, double,
            int64_t, double, int64_t, double, int64_t, double,
            int64_t, double, int64_t, double, double
        ) __asm__("sl_fn_6162695f70726f6265");
        int main(void) {
            return abi_probe(
                1, 1.0, 2, 2.0, 3, 3.0, 4, 4.0, 5, 5.0, 6, 6.0,
                7, 7.0, 8, 8.0, 9.0
            ) == 15 ? 0 : 1;
        }
        "#,
    )
    .unwrap();
    let status = Command::new("cc")
        .args(["-o"])
        .arg(&executable)
        .arg(&assembly_path)
        .arg(&runtime_path)
        .arg(&oracle_path)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(Command::new(&executable).status().unwrap().success());
    fs::remove_dir_all(directory).unwrap();
}

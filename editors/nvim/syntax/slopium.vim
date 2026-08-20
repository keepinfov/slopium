if exists("b:current_syntax")
  finish
endif

" `;;` above a declaration is documentation and is highlighted apart from an
" ordinary comment, so a file shows at a glance what carries prose (`D-134`).
" Declared first, because a later `match` does not win against an earlier one.
syntax match slopiumDoc ";;.*$" contains=slopiumTodo,@Spell
syntax match slopiumComment ";\%(;\)\@!.*$" contains=slopiumTodo,@Spell
syntax keyword slopiumTodo TODO FIXME NOTE SAFETY contained

syntax region slopiumString start=+"+ skip=+\\\\\|\\"+ end=+"+ contains=slopiumEscape
syntax match slopiumEscape +\\\%([nrt0"\\]\|x\x\x\)+ contained
syntax match slopiumNumber "\v(^|[[:space:](])\zs[-+]?(0[xX][0-9A-Fa-f_]+|0[bB][01_]+|\d[\d_]*(\.\d+)?([eE][+-]?\d+)?)\ze($|[[:space:])])"
syntax keyword slopiumBoolean true false
syntax match slopiumDelimiter "[()]"

" Broad lexical groups come first. Context-sensitive groups below override
" them, so `Choice` is a type, `choice` an identifier, and `(score ...)` a
" function call.
syntax match slopiumIdentifier "\<[a-z_][A-Za-z0-9_-]*\>"
syntax match slopiumNamedType "\<[A-Z][A-Za-z0-9_-]*\>"
syntax match slopiumCall "\%((\s*\)\@<=[A-Za-z_.+*/<>=-][A-Za-z0-9_:!.+*/<>=-]*"

" Declarations use nextgroup instead of a regexp beginning at `(`. This keeps
" the opening delimiter from masking `fn`, `enum`, and their declared names.
syntax keyword slopiumFunctionKeyword fn nextgroup=slopiumFunction skipwhite
syntax keyword slopiumLambdaKeyword lambda
syntax match slopiumFunction "[A-Za-z_][A-Za-z0-9_-]*" contained
syntax keyword slopiumTypeKeyword struct enum nextgroup=slopiumTypeName skipwhite
syntax match slopiumTypeName "[A-Z][A-Za-z0-9_-]*" contained
syntax keyword slopiumTestKeyword test
" An annotation is a list written between a declaration's keyword and its name.
" These two words are special only there — nothing reserves them elsewhere — so
" this highlights them wherever they appear, the way a reader reads them. A
" declaration that carries one does not highlight its name, because `nextgroup`
" is looking for the name and finds a `(`.
syntax keyword slopiumAnnotation inline deprecated target

syntax keyword slopiumBindingKeyword let nextgroup=slopiumModifier,slopiumVariableDefinition skipwhite
" A module-level name for a literal. It binds like `let` and is declared like a
" `fn`, so it takes the binding group and the declaration's `nextgroup`.
syntax keyword slopiumConstKeyword const nextgroup=slopiumVariableDefinition skipwhite
syntax keyword slopiumModifier mut contained nextgroup=slopiumVariableDefinition skipwhite
syntax match slopiumVariableDefinition "[a-z_][A-Za-z0-9_-]*" contained
syntax keyword slopiumAssignmentKeyword set nextgroup=slopiumVariable skipwhite
syntax match slopiumVariable "[a-z_][A-Za-z0-9_-]*" contained
syntax keyword slopiumControl do loop while break continue defer
" `unsafe` is a block like `do`, and a permission rather than a second type
" system. It gets its own group so that a reader scanning a file can find every
" place the compiler stopped proving things.
syntax keyword slopiumUnsafe unsafe
syntax keyword slopiumConditional if match when try and or
" Anchored after `(` so the `:as` of an import alias stays a field.
syntax match slopiumConversion "\%((\s*\)\@<=as\>"
" `export`, `take` and `extern` are the three words that name a boundary: the
" first two the module's, the third the C one. `extern` takes no `nextgroup`
" because what follows it is the foreign name as a string, not an identifier.
syntax keyword slopiumModuleKeyword export take extern

" A lowercase name followed by a type and `)` is a parameter/field
" declaration. Uses remain slopiumIdentifier, while the type has its own group.
syntax match slopiumParameter "\v<[a-z_][A-Za-z0-9_-]*>\ze\s+(&mut|&)?(unit|bool|i8|i16|i32|i64|u8|u16|u32|u64|f64|String|[A-Z][A-Za-z0-9_-]*)\s*\)"

syntax match slopiumOwnership "&mut\|&"
syntax keyword slopiumBuiltin clone list array slice len push get get-ref pop remove replace not bit-and bit-or bit-xor bit-not shl shr volatile-read volatile-write ptr-offset
syntax match slopiumOperator "\%((\s*\)\@<=\%([<>!]\=[=]\|[-+*/<>%]\)"
syntax keyword slopiumType unit bool i8 i16 i32 i64 u8 u16 u32 u64 f64 String List Array Slice Fn Ptr
syntax match slopiumEnumPath "\v<[A-Za-z_][A-Za-z0-9_-]*(:[A-Za-z_][A-Za-z0-9_-]*)+>"
syntax match slopiumField "\v:[A-Za-z_][A-Za-z0-9_-]*"
" A lone `:` is the type written after a value — `(let x 0 : u8)`. Anchored on
" whitespace so the `:` of a field or of a module path stays what it was.
syntax match slopiumAscription "\v\s:\ze\s"
syntax match slopiumArrow "->"
syntax match slopiumWildcard "\v<_>"

highlight default link slopiumComment Comment
highlight default link slopiumDoc SpecialComment
highlight default link slopiumTodo Todo
highlight default link slopiumString String
highlight default link slopiumEscape SpecialChar
highlight default link slopiumNumber Number
highlight default link slopiumBoolean Boolean
highlight default link slopiumIdentifier Identifier
highlight default link slopiumNamedType Type
highlight default link slopiumCall Function
highlight default link slopiumFunctionKeyword @keyword.function
highlight default link slopiumLambdaKeyword @keyword.function
highlight default link slopiumTypeKeyword @keyword.type
highlight default link slopiumTestKeyword @keyword
highlight default link slopiumAnnotation @attribute
highlight default link slopiumFunction @function
highlight default link slopiumTypeName @type.definition
highlight default link slopiumBindingKeyword @keyword
highlight default link slopiumAssignmentKeyword @keyword
highlight default link slopiumControl Conditional
highlight default link slopiumUnsafe @keyword.exception
highlight default link slopiumConditional @keyword.conditional
highlight default link slopiumConversion @keyword.operator
highlight default link slopiumModuleKeyword Include
highlight default link slopiumOwnership StorageClass
highlight default link slopiumBuiltin @function.builtin
highlight default link slopiumOperator Operator
highlight default link slopiumModifier StorageClass
highlight default link slopiumType @type.builtin
highlight default link slopiumParameter @variable.parameter
highlight default link slopiumVariableDefinition @variable
highlight default link slopiumVariable @variable
highlight default link slopiumEnumPath @constructor
highlight default link slopiumField @property
highlight default link slopiumConstKeyword @keyword
highlight default link slopiumAscription @keyword.operator
highlight default link slopiumArrow Operator
highlight default link slopiumWildcard Special
highlight default link slopiumDelimiter Delimiter

let b:current_syntax = "slopium"

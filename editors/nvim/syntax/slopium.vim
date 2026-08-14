if exists("b:current_syntax")
  finish
endif

syntax match slopiumComment ";.*$" contains=slopiumTodo,@Spell
syntax keyword slopiumTodo TODO FIXME NOTE SAFETY contained

syntax region slopiumString start=+"+ skip=+\\\\\|\\"+ end=+"+ contains=slopiumEscape
syntax match slopiumEscape +\\[nrt"\\]+ contained
syntax match slopiumNumber "\v(^|[[:space:](])\zs-?\d+(\.\d+)?([eE][+-]?\d+)?\ze($|[[:space:])])"
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

syntax keyword slopiumBindingKeyword let nextgroup=slopiumModifier,slopiumVariableDefinition skipwhite
syntax keyword slopiumModifier mut contained nextgroup=slopiumVariableDefinition skipwhite
syntax match slopiumVariableDefinition "[a-z_][A-Za-z0-9_-]*" contained
syntax keyword slopiumAssignmentKeyword set nextgroup=slopiumVariable skipwhite
syntax match slopiumVariable "[a-z_][A-Za-z0-9_-]*" contained
syntax keyword slopiumControl do loop while break continue
syntax keyword slopiumConditional if match try
" Anchored after `(` so the `:as` of an import alias stays a field.
syntax match slopiumConversion "\%((\s*\)\@<=as\>"
syntax keyword slopiumModuleKeyword export take

" A lowercase name followed by a type and `)` is a parameter/field
" declaration. Uses remain slopiumIdentifier, while the type has its own group.
syntax match slopiumParameter "\v<[a-z_][A-Za-z0-9_-]*>\ze\s+(&mut|&)?(unit|bool|i32|i64|f64|String|[A-Z][A-Za-z0-9_-]*)\s*\)"

syntax match slopiumOwnership "&mut\|&"
syntax keyword slopiumBuiltin clone list array slice len push get get-ref pop remove
syntax match slopiumOperator "\%((\s*\)\@<=[-+*/<>=]"
syntax keyword slopiumType unit bool i32 i64 f64 String List Array Slice Fn
syntax match slopiumEnumPath "\v<[A-Za-z_][A-Za-z0-9_-]*(:[A-Za-z_][A-Za-z0-9_-]*)+>"
syntax match slopiumField "\v:[A-Za-z_][A-Za-z0-9_-]*"
syntax match slopiumArrow "->"
syntax match slopiumWildcard "\v<_>"

highlight default link slopiumComment Comment
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
highlight default link slopiumFunction @function
highlight default link slopiumTypeName @type.definition
highlight default link slopiumBindingKeyword @keyword
highlight default link slopiumAssignmentKeyword @keyword
highlight default link slopiumControl Conditional
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
highlight default link slopiumArrow Operator
highlight default link slopiumWildcard Special
highlight default link slopiumDelimiter Delimiter

let b:current_syntax = "slopium"

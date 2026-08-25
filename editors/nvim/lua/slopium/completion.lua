local M = {}

local kinds = {
  Text = 1,
  Function = 3,
  Variable = 6,
  Keyword = 14,
  Snippet = 15,
  Struct = 22,
  Operator = 24,
  TypeParameter = 25,
}

local static_items = {
  { label = "fn", kind = kinds.Snippet, detail = "function declaration", insertText = "fn ${1:name} ((${2:arg} ${3:i64})) -> ${4:i64}\n  ${0:body}", insertTextFormat = 2 },
  { label = "test", kind = kinds.Snippet, detail = "test declaration", insertText = 'test "${1:name}"\n  ${0:true}', insertTextFormat = 2 },
  { label = "struct", kind = kinds.Snippet, detail = "struct declaration", insertText = "struct ${1:Name} ((${2:field} ${3:i64}))", insertTextFormat = 2 },
  { label = "enum", kind = kinds.Snippet, detail = "enum declaration", insertText = "enum ${1:Name}\n  ${0:Variant}", insertTextFormat = 2 },
  { label = "export", kind = kinds.Snippet, detail = "public module API", insertText = "export ${0:Name}", insertTextFormat = 2 },
  { label = "take", kind = kinds.Snippet, detail = "module import aliases", insertText = "take ${1:module} ${0:Name}", insertTextFormat = 2 },
  { label = "extern", kind = kinds.Snippet, detail = "declare a C function", insertText = 'extern "${1:c_name}" (${2:name} (${3:arg} ${4:i64})) -> ${0:i64}', insertTextFormat = 2 },
  { label = "const", kind = kinds.Snippet, detail = "module-level literal", insertText = "const ${1:name} ${0:value}", insertTextFormat = 2 },
  { label = "inline", kind = kinds.Snippet, detail = "annotation: worth copying into its callers", insertText = "inline", insertTextFormat = 2 },
  { label = "deprecated", kind = kinds.Snippet, detail = "annotation: warn at every use", insertText = 'deprecated "${0:use this instead}"', insertTextFormat = 2 },
  { label = "target", kind = kinds.Snippet, detail = "annotation: build this declaration only for one target", insertText = 'target "${0:x86_64-unknown-linux-gnu}"', insertTextFormat = 2 },
  { label = "let", kind = kinds.Snippet, detail = "immutable binding", insertText = "let ${1:name} ${0:value}", insertTextFormat = 2 },
  { label = "let mut", kind = kinds.Snippet, detail = "mutable binding", insertText = "let mut ${1:name} ${0:value}", insertTextFormat = 2 },
  { label = "set", kind = kinds.Snippet, detail = "assign a mutable binding or a field bound by a `&mut` match", insertText = "set ${1:name} ${0:value}", insertTextFormat = 2 },
  { label = "if", kind = kinds.Snippet, detail = "conditional expression", insertText = "if ${1:condition}\n    ${2:then}\n    ${0:else}", insertTextFormat = 2 },
  { label = "match", kind = kinds.Snippet, detail = "pattern match", insertText = "match ${1:value}\n    (${2:pattern} ${0:body})", insertTextFormat = 2 },
  { label = "when", kind = kinds.Snippet, detail = "one-sided conditional, and the guard of a match arm", insertText = "when ${1:condition}\n    ${0:body}", insertTextFormat = 2 },
  { label = ":", kind = kinds.Keyword, detail = "the type of the value before it" },
  { label = "lambda", kind = kinds.Snippet, detail = "closure over named captures", insertText = "lambda (${1:capture}) ((${2:arg} ${3:type})) -> ${4:type}\n    ${0:body}", insertTextFormat = 2 },
  { label = "do", kind = kinds.Keyword, detail = "expression sequence" },
  { label = "<<", kind = kinds.Operator, detail = "compose functions, right to left" },
  { label = ">>", kind = kinds.Operator, detail = "compose functions, left to right" },
  { label = "defer", kind = kinds.Snippet, detail = "run this when the scope ends, however it ends", insertText = "defer ${0:expression}", insertTextFormat = 2 },
  { label = "and", kind = kinds.Snippet, detail = "short-circuiting conjunction", insertText = "and ${1:left} ${0:right}", insertTextFormat = 2 },
  { label = "or", kind = kinds.Snippet, detail = "short-circuiting disjunction", insertText = "or ${1:left} ${0:right}", insertTextFormat = 2 },
  { label = "loop", kind = kinds.Snippet, detail = "unconditional loop", insertText = "loop\n    ${0:body}", insertTextFormat = 2 },
  { label = "while", kind = kinds.Snippet, detail = "conditional loop", insertText = "while ${1:condition}\n    ${0:body}", insertTextFormat = 2 },
  { label = "break", kind = kinds.Keyword },
  { label = "continue", kind = kinds.Keyword },
  { label = "try", kind = kinds.Snippet, detail = "propagate Result error", insertText = "try ${0:expression}", insertTextFormat = 2 },
  { label = "unsafe", kind = kinds.Snippet, detail = "raw pointer permission", insertText = "unsafe\n    ${0:body}", insertTextFormat = 2 },
  { label = "as", kind = kinds.Snippet, detail = "widen a number to a named type", insertText = "as ${1:i64} ${0:value}", insertTextFormat = 2 },
  { label = "mut", kind = kinds.Keyword, detail = "mutable marker" },
  { label = "true", kind = kinds.Keyword },
  { label = "false", kind = kinds.Keyword },
  { label = "_", kind = kinds.Keyword, detail = "wildcard pattern" },
  { label = "&", kind = kinds.Keyword, detail = "shared borrow: `&x`, or `(& x)`" },
  { label = "&mut", kind = kinds.Keyword, detail = "exclusive borrow: `&mut x`" },
  { label = "$", kind = kinds.Keyword, detail = "nest the rest of this form: `(a $ b c)` is `(a (b c))`" },
  { label = "unit", kind = kinds.TypeParameter },
  { label = "bool", kind = kinds.TypeParameter },
  { label = "i8", kind = kinds.TypeParameter },
  { label = "i16", kind = kinds.TypeParameter },
  { label = "i32", kind = kinds.TypeParameter },
  { label = "i64", kind = kinds.TypeParameter },
  { label = "u8", kind = kinds.TypeParameter },
  { label = "u16", kind = kinds.TypeParameter },
  { label = "u32", kind = kinds.TypeParameter },
  { label = "u64", kind = kinds.TypeParameter },
  { label = "f64", kind = kinds.TypeParameter },
  { label = "String", kind = kinds.TypeParameter },
  { label = "List", kind = kinds.TypeParameter },
  { label = "Array", kind = kinds.TypeParameter },
  { label = "Slice", kind = kinds.TypeParameter },
  { label = "Fn", kind = kinds.TypeParameter },
  { label = "Ptr", kind = kinds.TypeParameter },
  { label = "clone", kind = kinds.Function, detail = "structural clone" },
  { label = ".", kind = kinds.Function, detail = "field access" },
  { label = "list", kind = kinds.Function, detail = "construct List<T>" },
  { label = "array", kind = kinds.Function, detail = "construct Array<T, N>" },
  { label = "slice", kind = kinds.Function, detail = "borrow a collection range" },
  { label = "len", kind = kinds.Function, detail = "collection length" },
  { label = "push", kind = kinds.Function, detail = "push(&mut List<T>, T)" },
  { label = "get", kind = kinds.Function, detail = "get(&List<T>, i64) -> T" },
  { label = "get-ref", kind = kinds.Function, detail = "borrow an element" },
  { label = "remove", kind = kinds.Function, detail = "move an element out of List<T>" },
  { label = "replace", kind = kinds.Function, detail = "replace(&mut List<T>, i64, T) -> T" },
  { label = "pop", kind = kinds.Function, detail = "pop(&mut List<T>) -> Option<T>" },
  { label = "+", kind = kinds.Function, detail = "numeric addition" },
  { label = "-", kind = kinds.Function, detail = "numeric subtraction" },
  { label = "*", kind = kinds.Function, detail = "numeric multiplication" },
  { label = "/", kind = kinds.Function, detail = "numeric division" },
  { label = "<", kind = kinds.Function, detail = "less than" },
  { label = ">", kind = kinds.Function, detail = "greater than" },
  { label = "=", kind = kinds.Function, detail = "equality" },
  { label = "%", kind = kinds.Function, detail = "integer remainder" },
  { label = "<=", kind = kinds.Function, detail = "less than or equal" },
  { label = ">=", kind = kinds.Function, detail = "greater than or equal" },
  { label = "!=", kind = kinds.Function, detail = "inequality" },
  { label = "not", kind = kinds.Function, detail = "not(bool) -> bool" },
  { label = "bit-and", kind = kinds.Function, detail = "bitwise and" },
  { label = "bit-or", kind = kinds.Function, detail = "bitwise or" },
  { label = "bit-xor", kind = kinds.Function, detail = "bitwise exclusive or" },
  { label = "bit-not", kind = kinds.Function, detail = "bitwise complement" },
  { label = "shl", kind = kinds.Function, detail = "left shift" },
  { label = "shr", kind = kinds.Function, detail = "right shift, arithmetic" },
  { label = "volatile-read", kind = kinds.Function, detail = "volatile-read((Ptr T)) -> T" },
  { label = "volatile-write", kind = kinds.Function, detail = "volatile-write((Ptr T) T) -> unit" },
  { label = "ptr-offset", kind = kinds.Function, detail = "ptr-offset((Ptr T) u64) -> (Ptr T)" },
}

-- The words the language reserves rather than defines. A program cannot
-- introduce one (`SL0101`), so nothing scanned from a buffer may offer one
-- back as a name to use.
local reserved = {
  ["async"] = true,
  ["await"] = true,
  ["for"] = true,
  ["format"] = true,
  ["macro"] = true,
  ["define-syntax"] = true,
  ["usize"] = true,
  ["isize"] = true,
  ["f32"] = true,
}

local function add(items, seen, label, kind, detail)
  if not label or label == "" or seen[label] or reserved[label] then
    return
  end
  seen[label] = true
  table.insert(items, {
    label = label,
    kind = kind,
    detail = detail,
  })
end

function M.items(bufnr)
  bufnr = bufnr or vim.api.nvim_get_current_buf()
  local items = vim.deepcopy(static_items)
  local seen = {}
  for _, item in ipairs(items) do
    seen[item.label] = true
  end

  local source = table.concat(vim.api.nvim_buf_get_lines(bufnr, 0, -1, false), "\n")
  for name in source:gmatch("%(%s*fn%s+([%a_][%w_-]*)") do
    add(items, seen, name, kinds.Function, "function in this buffer")
  end
  for name in source:gmatch("%(%s*struct%s+([%a_][%w_-]*)") do
    add(items, seen, name, kinds.Struct, "struct in this buffer")
  end
  for name in source:gmatch("%(%s*enum%s+([%a_][%w_-]*)") do
    add(items, seen, name, kinds.Struct, "enum in this buffer")
  end
  for parameters in source:gmatch(
    "%(%s*fn%s+[%a_][%w_-]*%s+%((.-)%)%s*%-%>"
  ) do
    for name in parameters:gmatch("%(%s*([%a_][%w_-]*)%s+") do
      add(items, seen, name, kinds.Variable, "function parameter")
    end
  end
  for name in source:gmatch("%(%s*let%s+mut%s+([%a_][%w_-]*)") do
    add(items, seen, name, kinds.Variable, "mutable binding")
  end
  for name in source:gmatch("%(%s*let%s+([%a_][%w_-]*)") do
    if name ~= "mut" then
      add(items, seen, name, kinds.Variable, "binding")
    end
  end

  return items
end

local Source = {}
Source.__index = Source

function Source.new()
  return setmetatable({}, Source)
end

function Source:is_available()
  return vim.bo.filetype == "slopium"
end

function Source:get_debug_name()
  return "slopium"
end

function Source:get_keyword_pattern()
  return [[\%([[:alnum:]_&:.+*/<>=-]\+\)]]
end

-- While `slopium-lsp` is attached and `cmp_nvim_lsp` carries its items into
-- the popup, the scanner steps back to the snippets the server does not have;
-- every other word it offered is offered by the server itself. Without either
-- half, the full scan completes as before.
function Source:complete(params, callback)
  if require("slopium.lsp").attached(params.context.bufnr) and pcall(require, "cmp_nvim_lsp") then
    local items = {}
    for _, item in ipairs(static_items) do
      if item.insertTextFormat == 2 then
        table.insert(items, vim.deepcopy(item))
      end
    end
    callback({ items = items, isIncomplete = false })
    return
  end
  callback({
    items = M.items(params.context.bufnr),
    isIncomplete = false,
  })
end

function M.source()
  return Source.new()
end

function M.omnifunc(findstart, base)
  if require("slopium.lsp").attached(0) then
    return vim.lsp.omnifunc(findstart, base)
  end
  if findstart == 1 then
    local line = vim.api.nvim_get_current_line()
    local column = vim.fn.col(".") - 1
    while column > 0 and line:sub(column, column):match("[%w_&:.+*/<>=%%!-]") do
      column = column - 1
    end
    return column
  end

  local matches = {}
  for _, item in ipairs(M.items()) do
    if item.label:sub(1, #base) == base then
      table.insert(matches, {
        word = item.label,
        abbr = item.label,
        menu = item.detail and ("[" .. item.detail .. "]") or "[Slopium]",
      })
    end
  end
  return matches
end

return M

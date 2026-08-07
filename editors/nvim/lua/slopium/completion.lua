local M = {}

local kinds = {
  Text = 1,
  Function = 3,
  Variable = 6,
  Keyword = 14,
  Snippet = 15,
  Struct = 22,
  TypeParameter = 25,
}

local static_items = {
  { label = "fn", kind = kinds.Snippet, detail = "function declaration", insertText = "fn ${1:name} ((${2:arg} ${3:i64})) -> ${4:i64}\n  ${0:body}", insertTextFormat = 2 },
  { label = "test", kind = kinds.Snippet, detail = "test declaration", insertText = 'test "${1:name}"\n  ${0:true}', insertTextFormat = 2 },
  { label = "struct", kind = kinds.Snippet, detail = "struct declaration", insertText = "struct ${1:Name} ((${2:field} ${3:i64}))", insertTextFormat = 2 },
  { label = "enum", kind = kinds.Snippet, detail = "enum declaration", insertText = "enum ${1:Name}\n  ${0:Variant}", insertTextFormat = 2 },
  { label = "export", kind = kinds.Snippet, detail = "public module API", insertText = "export ${0:Name}", insertTextFormat = 2 },
  { label = "take", kind = kinds.Snippet, detail = "module import aliases", insertText = "take ${1:module} ${0:Name}", insertTextFormat = 2 },
  { label = "let", kind = kinds.Snippet, detail = "immutable binding", insertText = "let ${1:name} ${0:value}", insertTextFormat = 2 },
  { label = "let mut", kind = kinds.Snippet, detail = "mutable binding", insertText = "let mut ${1:name} ${0:value}", insertTextFormat = 2 },
  { label = "set", kind = kinds.Snippet, detail = "assign a mutable binding", insertText = "set ${1:name} ${0:value}", insertTextFormat = 2 },
  { label = "if", kind = kinds.Snippet, detail = "conditional expression", insertText = "if ${1:condition}\n    ${2:then}\n    ${0:else}", insertTextFormat = 2 },
  { label = "match", kind = kinds.Snippet, detail = "pattern match", insertText = "match ${1:value}\n    (${2:pattern} ${0:body})", insertTextFormat = 2 },
  { label = "do", kind = kinds.Keyword, detail = "expression sequence" },
  { label = "loop", kind = kinds.Snippet, detail = "unconditional loop", insertText = "loop\n    ${0:body}", insertTextFormat = 2 },
  { label = "while", kind = kinds.Snippet, detail = "conditional loop", insertText = "while ${1:condition}\n    ${0:body}", insertTextFormat = 2 },
  { label = "break", kind = kinds.Keyword },
  { label = "continue", kind = kinds.Keyword },
  { label = "try", kind = kinds.Snippet, detail = "propagate Result error", insertText = "try ${0:expression}", insertTextFormat = 2 },
  { label = "as", kind = kinds.Snippet, detail = "widen a number to a named type", insertText = "as ${1:i64} ${0:value}", insertTextFormat = 2 },
  { label = "mut", kind = kinds.Keyword, detail = "mutable marker" },
  { label = "true", kind = kinds.Keyword },
  { label = "false", kind = kinds.Keyword },
  { label = "_", kind = kinds.Keyword, detail = "wildcard pattern" },
  { label = "&", kind = kinds.Keyword, detail = "shared borrow" },
  { label = "&mut", kind = kinds.Keyword, detail = "exclusive borrow" },
  { label = "unit", kind = kinds.TypeParameter },
  { label = "bool", kind = kinds.TypeParameter },
  { label = "i32", kind = kinds.TypeParameter },
  { label = "i64", kind = kinds.TypeParameter },
  { label = "f64", kind = kinds.TypeParameter },
  { label = "String", kind = kinds.TypeParameter },
  { label = "List", kind = kinds.TypeParameter },
  { label = "Array", kind = kinds.TypeParameter },
  { label = "Slice", kind = kinds.TypeParameter },
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
  { label = "pop", kind = kinds.Function, detail = "pop(&mut List<T>) -> Option<T>" },
  { label = "+", kind = kinds.Function, detail = "numeric addition" },
  { label = "-", kind = kinds.Function, detail = "numeric subtraction" },
  { label = "*", kind = kinds.Function, detail = "numeric multiplication" },
  { label = "/", kind = kinds.Function, detail = "numeric division" },
  { label = "<", kind = kinds.Function, detail = "less than" },
  { label = ">", kind = kinds.Function, detail = "greater than" },
  { label = "=", kind = kinds.Function, detail = "equality" },
}

local function add(items, seen, label, kind, detail)
  if not label or label == "" or seen[label] then
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

function Source:complete(params, callback)
  callback({
    items = M.items(params.context.bufnr),
    isIncomplete = false,
  })
end

function M.source()
  return Source.new()
end

function M.omnifunc(findstart, base)
  if findstart == 1 then
    local line = vim.api.nvim_get_current_line()
    local column = vim.fn.col(".") - 1
    while column > 0 and line:sub(column, column):match("[%w_&:.+*/<>=-]") do
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

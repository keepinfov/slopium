-- The indent computation `indent/slopium.lua` installs.
--
-- It is the Lisp rule with one addition: `|)` closes every list still open, so
-- the depth it leaves is zero and the next declaration starts at the margin
-- (`D-151`). Everything else follows the layout `slopium fmt` produces — a body
-- under a body-carrying form is one `shiftwidth` in, and the arguments of
-- anything else align under the first one.

local M = {}

--- Every form that carries a body, taken from `lispwords` so that the editor
--- keeps one list rather than three.
local function body_forms()
  local words = {}
  for word in vim.bo.lispwords:gmatch("[^,]+") do
    words[word] = true
  end
  return words
end

--- The open lists above `stop`, innermost last.
---
--- Each entry is the column the `(` sits in, the head written after it, and the
--- column of the first element after that head when one shares the line.
local function open_lists(from, stop)
  local stack = {}
  for lnum = from, stop - 1 do
    local line = vim.fn.getline(lnum)
    local column = 0
    local index = 1
    while index <= #line do
      local char = line:sub(index, index)
      if char == ";" then
        break
      elseif char == '"' then
        index = index + 1
        while index <= #line do
          local inner = line:sub(index, index)
          if inner == "\\" then
            index = index + 1
          elseif inner == '"' then
            break
          end
          index = index + 1
        end
      elseif char == "(" then
        table.insert(stack, { column = column, head = nil, first = nil })
      elseif char == "|" and line:sub(index + 1, index + 1) == ")" then
        -- The closer: every list a declaration left open ends here.
        stack = {}
        index = index + 1
        column = column + 1
      elseif char == ")" then
        table.remove(stack)
      elseif not char:match("%s") then
        -- An atom. Whichever list it lands in learns what it is: the head if
        -- the list has none yet, the first argument if it has.
        local finish = index
        while finish <= #line do
          local next_char = line:sub(finish, finish)
          if next_char:match("[%s()\";|]") then
            break
          end
          finish = finish + 1
        end
        local top = stack[#stack]
        if top then
          if not top.head then
            top.head = line:sub(index, finish - 1)
          elseif not top.first then
            top.first = column
          end
        end
        column = column + (finish - index) - 1
        index = finish - 1
      end
      index = index + 1
      column = column + 1
    end
  end
  return stack
end

--- The first line of the declaration `lnum` is inside.
---
--- Every declaration starts at the margin, which is what `slopium fmt`
--- guarantees, so the search is for the nearest line that begins with `(`.
local function declaration_start(lnum)
  for candidate = lnum - 1, 1, -1 do
    local line = vim.fn.getline(candidate)
    if line:sub(1, 1) == "(" or line:sub(1, 1) == ";" then
      return candidate
    end
  end
  return 1
end

function M.indent()
  local lnum = vim.v.lnum
  local line = vim.fn.getline(lnum)
  local trimmed = line:gsub("^%s+", "")
  if trimmed == "" then
    return -1
  end
  local stack = open_lists(declaration_start(lnum), lnum)
  -- A line that begins by closing belongs to the list it closes.
  if trimmed:sub(1, 2) == "|)" then
    return stack[1] and stack[1].column or 0
  end
  if trimmed:sub(1, 1) == ")" then
    table.remove(stack)
  end
  local top = stack[#stack]
  if not top then
    return 0
  end
  if not top.head or body_forms()[top.head] then
    return top.column + vim.bo.shiftwidth
  end
  if top.first then
    return top.first
  end
  return top.column + 1 + #top.head + 1
end

return M

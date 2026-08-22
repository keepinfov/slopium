-- Indentation for Slopium.
--
-- Vim's built-in Lisp indenter counts `(` and `)` in Vim's own C source and
-- cannot be taught another closer, so a file holding `|)` — the token that
-- closes every list a declaration left open (`D-151`) — indents as if that
-- declaration never ended. This replaces it.
--
-- The list of body-carrying forms is `lispwords`, read from the buffer rather
-- than copied here: `ftplugin/slopium.lua` sets it, the language server checks
-- it against the words the language has, and one list is the whole point.

if vim.b.did_indent then
  return
end
vim.b.did_indent = true

-- `indentexpr` is used in place of `lisp`, so the built-in indenter is off from
-- here on. `lispwords` stays, because this is what reads it.
vim.bo.indentexpr = "v:lua.require'slopium.indent'.indent()"
vim.bo.indentkeys = "!^F,o,O,0),0|"

vim.b.undo_indent = "setlocal indentexpr< indentkeys<"

vim.bo.commentstring = "; %s"
vim.bo.comments = ":;"
vim.bo.expandtab = true
vim.bo.shiftwidth = 2
vim.bo.softtabstop = 2
vim.bo.tabstop = 2
-- Vim's built-in Lisp indenter counts `(` and `)` in Vim's own C source and
-- cannot be taught `|)`, the token that closes every list a declaration left
-- open (`D-151`), so `indent/slopium.lua` replaces it and this stays off.
vim.bo.lisp = false
vim.bo.autoindent = true
vim.bo.iskeyword = vim.bo.iskeyword .. ",-"
-- Every form that carries a body, so that the body indents under the word
-- rather than under the operands. `lambda` and `unsafe` arrived after this line
-- was first written and indent like `do` and `loop` do. It stays in `lispwords`
-- rather than in the indent module because one list is the point: the language
-- server checks this one against the words the language has.
vim.bo.lispwords =
  "fn,test,struct,enum,const,export,take,extern,lambda,let,set,do,defer,if,match,when,loop,while,unsafe,list,array,push"
vim.bo.omnifunc = "v:lua.slopium_omnifunc"

vim.b.undo_ftplugin =
  "setlocal commentstring< comments< expandtab< shiftwidth< softtabstop< tabstop< lisp< autoindent< iskeyword< lispwords< omnifunc<"

vim.bo.commentstring = "; %s"
vim.bo.comments = ":;"
vim.bo.expandtab = true
vim.bo.shiftwidth = 2
vim.bo.softtabstop = 2
vim.bo.tabstop = 2
vim.bo.lisp = true
vim.bo.autoindent = true
vim.bo.iskeyword = vim.bo.iskeyword .. ",-"
-- Every form that carries a body, so that `lisp` indents the body under the
-- word rather than under the operands. `lambda` and `unsafe` arrived after this
-- line was first written and indent like `do` and `loop` do.
vim.bo.lispwords =
  "fn,test,struct,enum,export,take,extern,lambda,let,set,do,if,match,loop,while,unsafe,list,array,push"
vim.bo.omnifunc = "v:lua.slopium_omnifunc"

vim.b.undo_ftplugin =
  "setlocal commentstring< comments< expandtab< shiftwidth< softtabstop< tabstop< lisp< autoindent< iskeyword< lispwords< omnifunc<"

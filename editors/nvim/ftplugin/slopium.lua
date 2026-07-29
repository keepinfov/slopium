vim.bo.commentstring = "; %s"
vim.bo.comments = ":;"
vim.bo.expandtab = true
vim.bo.shiftwidth = 2
vim.bo.softtabstop = 2
vim.bo.tabstop = 2
vim.bo.lisp = true
vim.bo.autoindent = true
vim.bo.iskeyword = vim.bo.iskeyword .. ",-"
vim.bo.lispwords =
  "fn,test,struct,enum,export,take,let,set,do,if,match,loop,while,list,array,push,println,print"
vim.bo.omnifunc = "v:lua.slopium_omnifunc"

vim.b.undo_ftplugin =
  "setlocal commentstring< comments< expandtab< shiftwidth< softtabstop< tabstop< lisp< autoindent< iskeyword< lispwords< omnifunc<"

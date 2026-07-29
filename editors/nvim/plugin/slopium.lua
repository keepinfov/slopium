if vim.g.loaded_slopium_nvim then
  return
end
vim.g.loaded_slopium_nvim = true

require("slopium").setup()

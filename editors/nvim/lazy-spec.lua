local source = debug.getinfo(1, "S").source:sub(2)
local real_source = (vim.uv or vim.loop).fs_realpath(source) or source
local plugin_dir = vim.fs.dirname(real_source)

return {
  {
    dir = plugin_dir,
    name = "slopium.nvim",
    lazy = false,
    config = function()
      require("slopium").setup()
    end,
  },
}

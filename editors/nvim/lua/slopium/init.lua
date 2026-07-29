local M = {}

local configured = false
local cmp_registered = false

local function setup_cmp()
  if cmp_registered then
    return true
  end
  local ok, cmp = pcall(require, "cmp")
  if not ok then
    return false
  end
  cmp.register_source("slopium", require("slopium.completion").source())
  cmp.setup.filetype("slopium", {
    sources = cmp.config.sources({
      { name = "slopium", priority = 1000 },
      { name = "buffer", priority = 500 },
    }),
  })
  cmp_registered = true
  return true
end

function M.setup(opts)
  opts = opts or {}
  require("slopium.diagnostics").setup(opts)
  require("slopium.lsp").setup(opts)
  _G.slopium_omnifunc = require("slopium.completion").omnifunc

  vim.filetype.add({
    extension = {
      slp = "slopium",
    },
  })

  if not configured then
    configured = true
    local group = vim.api.nvim_create_augroup("slopium_nvim", { clear = true })
    vim.api.nvim_create_autocmd("FileType", {
      group = group,
      pattern = "slopium",
      callback = function(event)
        if not require("slopium.lsp").attach(event.buf) then
          require("slopium.diagnostics").attach(event.buf)
        end
        setup_cmp()
      end,
    })
    vim.api.nvim_create_autocmd("User", {
      group = group,
      pattern = "LazyDone",
      callback = setup_cmp,
    })
  end

  setup_cmp()
  if vim.bo.filetype == "slopium" then
    if not require("slopium.lsp").attach(0) then
      require("slopium.diagnostics").attach(0)
    end
  end
end

return M

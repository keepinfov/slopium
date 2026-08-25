local M = {}

local options = {}

local function plugin_repo()
  local source = debug.getinfo(1, "S").source:sub(2)
  local directory = vim.fs.dirname(source)
  for _ = 1, 4 do
    directory = vim.fs.dirname(directory)
  end
  return directory
end

local function sibling_lsp(path)
  if not path or path == "" then
    return nil
  end
  local candidate = vim.fs.dirname(path) .. "/slopium-lsp"
  if vim.fn.executable(candidate) == 1 then
    return candidate
  end
  return nil
end

function M.executable()
  if options.lsp and options.lsp ~= "" and vim.fn.executable(vim.fn.expand(options.lsp)) == 1 then
    return vim.fn.expand(options.lsp)
  end
  if vim.g.slopium_lsp and vim.g.slopium_lsp ~= ""
      and vim.fn.executable(vim.fn.expand(vim.g.slopium_lsp)) == 1 then
    return vim.fn.expand(vim.g.slopium_lsp)
  end
  local compiler = options.compiler or vim.g.slopium_compiler or vim.fn.exepath("slopic")
  local sibling = sibling_lsp(compiler)
  if sibling then
    return sibling
  end
  local installed = vim.fn.exepath("slopium-lsp")
  if installed ~= "" then
    return installed
  end
  for _, profile in ipairs({ "debug", "release" }) do
    local candidate = plugin_repo() .. "/target/" .. profile .. "/slopium-lsp"
    if vim.fn.executable(candidate) == 1 then
      return candidate
    end
  end
  return nil
end

function M.attach(bufnr)
  bufnr = bufnr or vim.api.nvim_get_current_buf()
  local executable = M.executable()
  if not executable then
    return false
  end
  local root = vim.fs.root(bufnr, { "Slopium.toml", ".git" })
    or vim.fs.dirname(vim.api.nvim_buf_get_name(bufnr))
  local capabilities = vim.lsp.protocol.make_client_capabilities()
  local has_cmp, cmp_lsp = pcall(require, "cmp_nvim_lsp")
  if has_cmp then
    capabilities = cmp_lsp.default_capabilities(capabilities)
  end
  local client_id = vim.lsp.start({
    name = "slopium-lsp",
    cmd = { executable },
    root_dir = root,
    capabilities = capabilities,
  }, { bufnr = bufnr })
  return client_id ~= nil
end

function M.attached(bufnr)
  bufnr = bufnr or vim.api.nvim_get_current_buf()
  return #vim.lsp.get_clients({ bufnr = bufnr, name = "slopium-lsp" }) > 0
end

function M.setup(opts)
  options = opts or {}
end

return M

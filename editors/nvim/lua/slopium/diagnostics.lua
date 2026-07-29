local M = {}

local namespace = vim.api.nvim_create_namespace("slopium")
local jobs = {}
local options = {}

local function plugin_repo()
  local source = debug.getinfo(1, "S").source:sub(2)
  local directory = vim.fs.dirname(source)
  for _ = 1, 4 do
    directory = vim.fs.dirname(directory)
  end
  return directory
end

local function compiler()
  if options.compiler and options.compiler ~= "" then
    return vim.fn.expand(options.compiler)
  end
  if vim.g.slopium_compiler and vim.g.slopium_compiler ~= "" then
    return vim.fn.expand(vim.g.slopium_compiler)
  end
  local executable = vim.fn.exepath("slopic")
  if executable ~= "" then
    return executable
  end
  for _, profile in ipairs({ "debug", "release" }) do
    local candidate = plugin_repo() .. "/target/" .. profile .. "/slopic"
    if vim.fn.executable(candidate) == 1 then
      return candidate
    end
  end
  return nil
end

local function parse(lines)
  local diagnostics = {}
  for _, line in ipairs(lines) do
    if line ~= "" then
      local ok, value = pcall(vim.json.decode, line)
      if ok and value and value.span then
        local message = value.message
        if type(value.code) == "string" then
          message = "[" .. value.code .. "] " .. message
        end
        if type(value.help) == "string" then
          message = message .. "\nhelp: " .. value.help
        end
        for _, label in ipairs(value.labels or {}) do
          if type(label.message) == "string" then
            message = message .. "\nlabel: " .. label.message
          end
        end
        for _, note in ipairs(value.notes or {}) do
          message = message .. "\nnote: " .. tostring(note)
        end
        for _, suggestion in ipairs(value.suggestions or {}) do
          if type(suggestion.message) == "string" and type(suggestion.replacement) == "string" then
            message = message
              .. "\nsuggestion: "
              .. suggestion.message
              .. ": `"
              .. suggestion.replacement
              .. "`"
          end
        end
        local column = math.max((value.span.column or 1) - 1, 0)
        local width = math.max((value.span["end"] or 0) - (value.span.start or 0), 1)
        table.insert(diagnostics, {
          lnum = math.max((value.span.line or 1) - 1, 0),
          col = column,
          end_col = column + width,
          severity = value.severity == "warning"
              and vim.diagnostic.severity.WARN
            or vim.diagnostic.severity.ERROR,
          source = "slopic",
          message = message,
        })
      end
    end
  end
  return diagnostics
end

function M.check(bufnr, notify_missing)
  bufnr = bufnr or vim.api.nvim_get_current_buf()
  if not vim.api.nvim_buf_is_valid(bufnr) or vim.bo[bufnr].filetype ~= "slopium" then
    return
  end
  local path = vim.api.nvim_buf_get_name(bufnr)
  if path == "" or vim.bo[bufnr].modified or vim.fn.filereadable(path) ~= 1 then
    return
  end
  local executable = compiler()
  if not executable then
    if notify_missing then
      vim.notify(
        "slopic not found: enter `nix shell .`, build the repository, or set vim.g.slopium_compiler",
        vim.log.levels.WARN
      )
    end
    return
  end

  if jobs[bufnr] then
    vim.fn.jobstop(jobs[bufnr])
  end
  local stderr = {}
  jobs[bufnr] = vim.fn.jobstart({
    executable,
    path,
    "--emit",
    "check",
    "--diagnostic-format",
    "json",
  }, {
    stderr_buffered = true,
    on_stderr = function(_, data)
      vim.list_extend(stderr, data or {})
    end,
    on_exit = function(job_id)
      if jobs[bufnr] ~= job_id then
        return
      end
      jobs[bufnr] = nil
      vim.schedule(function()
        if vim.api.nvim_buf_is_valid(bufnr) then
          vim.diagnostic.set(namespace, bufnr, parse(stderr), {})
        end
      end)
    end,
  })
end

function M.attach(bufnr)
  bufnr = bufnr or vim.api.nvim_get_current_buf()
  if vim.b[bufnr].slopium_diagnostics_attached then
    return
  end
  vim.b[bufnr].slopium_diagnostics_attached = true
  vim.api.nvim_buf_create_user_command(bufnr, "SlopiumCheck", function()
    M.check(bufnr, true)
  end, { desc = "Check this file with slopic" })
  vim.api.nvim_create_autocmd("BufWritePost", {
    buffer = bufnr,
    callback = function()
      M.check(bufnr, false)
    end,
    desc = "Refresh Slopium diagnostics",
  })
  vim.schedule(function()
    M.check(bufnr, false)
  end)
end

function M.setup(opts)
  options = opts or {}
end

return M

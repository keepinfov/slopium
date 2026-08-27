#!/usr/bin/env bash
set -euo pipefail

# The shipped Neovim plugin has logic now, and this runs it.
#
# `|)` closes every list a declaration left open (`D-151`), and Vim's built-in
# Lisp indenter counts `(` and `)` in Vim's own C source: it cannot be taught
# another closer, so a file holding one indents as if that declaration never
# ended. `editors/nvim/indent/slopium.lua` replaces it, and this checks that it
# does — in Neovim, over the bundled library, rather than by reading the Lua.
#
# `SLOPIUM_STRICT=1` turns a skip into a failure. A machine that quietly lacks a
# tool otherwise reports a green check that verified nothing.
skip() {
  echo "nvim-check: $1" >&2
  if [ -n "${SLOPIUM_STRICT:-}" ]; then
    echo "nvim-check: SLOPIUM_STRICT is set; a skipped check is a failed one" >&2
    exit 1
  fi
  exit 0
}

fail() {
  echo "nvim-check: $1" >&2
  exit 1
}

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
check_dir="$(mktemp -d)"
trap 'rm -rf "$check_dir"' EXIT

command -v nvim >/dev/null 2>&1 || skip "no nvim; skipping the editor plugin checks"

cat >"$check_dir/init.lua" <<LUA
vim.opt.runtimepath:prepend("$workspace_dir/editors/nvim")
vim.opt.swapfile = false
vim.cmd("filetype plugin indent on")
LUA

# Reindent the whole buffer and write it back beside the original.
cat >"$check_dir/reindent.lua" <<'LUA'
if vim.bo.filetype ~= "slopium" then
  io.stderr:write("filetype is `" .. vim.bo.filetype .. "` rather than `slopium`\n")
  vim.cmd("cq!")
end
if vim.bo.indentexpr == "" then
  io.stderr:write("no indentexpr: the built-in Lisp indenter is still in charge\n")
  vim.cmd("cq!")
end
vim.cmd("normal! gg=G")
vim.cmd("write! " .. vim.fn.expand("%:p") .. ".indented")
vim.cmd("qa!")
LUA

reindent() {
  nvim --headless -u "$check_dir/init.lua" -S "$check_dir/reindent.lua" "$1" \
    >/dev/null 2>"$check_dir/nvim.stderr" ||
    {
      cat "$check_dir/nvim.stderr" >&2
      fail "nvim exited non-zero on \`$1\`"
    }
}

# A declaration starts at the margin, and that is the property `|)` threatens:
# an indenter that does not know the closer leaves the depth open and pushes
# every following declaration to the right, one level per closer it missed.
sources=0
while IFS= read -r -d '' source; do
  name="$(basename "$source")"
  cp "$source" "$check_dir/$name"
  reindent "$check_dir/$name"
  # Only the margin is asserted over the whole library: an editor indents a
  # continuation line without measuring what a whole form would cost, so it may
  # differ from `fmt` there, and does on two lines of the tree.
  moved="$(
    awk 'NR == FNR { before[FNR] = $0; next }
         before[FNR] ~ /^[^ \t]/ && $0 ~ /^[ \t]/ { print FNR ": " before[FNR] }' \
      "$source" "$check_dir/$name.indented"
  )"
  if [ -n "$moved" ]; then
    echo "nvim-check: reindenting \`$source\` moved a declaration off the margin" >&2
    printf '%s\n' "$moved" >&2
    exit 1
  fi
  # Whatever it decided, it has to decide the same thing twice. The copy keeps
  # the name it had, because the filetype is read off the extension.
  mkdir -p "$check_dir/again"
  cp "$check_dir/$name.indented" "$check_dir/again/$name"
  reindent "$check_dir/again/$name"
  cmp --silent "$check_dir/$name.indented" "$check_dir/again/$name.indented" ||
    fail "indentation of \`$source\` is not idempotent"
  sources=$((sources + 1))
done < <(find "$workspace_dir/std" -name '*.slp' -print0 | sort -z)

echo "nvim-check: $sources bundled modules keep every declaration at the margin ... ok"

# The cases the indenter is written for, spelled out. Each line says what column
# it belongs in, so a wrong answer names itself.
cat >"$check_dir/cases.slp" <<'SLOPIUM'
(fn first ((value i64)) -> i64
  (match value
    (0 "none")
    (_ (if (> value 10)
         "many"
         "some"|)

(fn second ((text &String)) -> i64
  (let width (len text))
  (println-i64 $ + width 1)
  width)

(fn third () -> i64
  (fold entries
        start
        step))
SLOPIUM
cp "$check_dir/cases.slp" "$check_dir/cases.expected"
reindent "$check_dir/cases.slp"
diff -u "$check_dir/cases.expected" "$check_dir/cases.slp.indented" >/dev/null ||
  fail "the indenter disagrees with the layout on a case it is written for"
echo "nvim-check: the closer, a body and an aligned call indent as written ... ok"

# Completing a documented function has to reach the popup with what hover
# already shows: the full signature and the `;;` block above the declaration.
# Both roads there are held against one fixture — the reply a language server
# sends, and what the plugin's scanner answers when no server exists.
version="$(grep -m1 '^version = ' "$workspace_dir/Cargo.toml" | cut -d\" -f2)"
[ -n "$version" ] || fail "cannot read the workspace version for the fixture"

mkdir -p "$check_dir/completion/src"
cat >"$check_dir/completion/Slopium.toml" <<TOML
[package]
name = "popup"
version = "$version"
entry = "src/main.slp"
source = "src"
TOML
cat >"$check_dir/completion/src/main.slp" <<'SLOPIUM'
;; Doubles a value by adding it to itself.
(fn twice ((value i64)) -> i64 (+ value value))

(fn main () -> i32 0)
SLOPIUM

server="${SLOPIUM_LSP:-$workspace_dir/target/debug/slopium-lsp}"
if [ ! -x "$server" ]; then
  echo "nvim-check: building slopium-lsp for the completion checks"
  cargo build -q -p slopium-lsp --manifest-path "$workspace_dir/Cargo.toml"
fi

# Through a real client over stdio: the reply to `textDocument/completion`
# carries the typed parameters and, as markdown, the sentence written above
# the declaration.
cat >"$check_dir/server-completion.lua" <<'LUA'
local project = vim.env.SLOPIUM_COMPLETION_PROJECT
vim.cmd("edit " .. project .. "/src/main.slp")
assert(vim.bo.filetype == "slopium", "filetype did not become `slopium`")
local bufnr = vim.api.nvim_get_current_buf()

local published = false
local client_id = vim.lsp.start({
  name = "slopium-lsp",
  cmd = { vim.env.SLOPIUM_LSP_SERVER },
  root_dir = project,
  handlers = {
    ["textDocument/publishDiagnostics"] = function()
      published = true
      return true
    end,
  },
}, { bufnr = bufnr })
assert(client_id, "the language server did not start")
if not vim.wait(30000, function()
  return published
end) then
  error("the server never published after the buffer was opened")
end

local client = assert(vim.lsp.get_client_by_id(client_id))
local response = client:request_sync("textDocument/completion", {
  textDocument = { uri = vim.uri_from_bufnr(bufnr) },
  position = { line = 3, character = 8 },
}, 30000)
assert(response.err == nil, "completion failed: " .. tostring(response.err))

local twice
for _, item in ipairs(response.result.items) do
  if item.label == "twice" then
    twice = item
    break
  end
end
assert(twice, "the server did not offer `twice`")
assert(
  twice.detail == "fn twice(i64) -> i64",
  "the server's detail was `" .. tostring(twice.detail) .. "`"
)
assert(
  type(twice.documentation) == "table" and twice.documentation.kind == "markdown",
  "the server sent no markdown documentation window"
)
for _, part in ipairs({ "Doubles a value by adding it to itself.", "fn twice(i64) -> i64" }) do
  assert(
    twice.documentation.value:find(part, 1, true),
    "the documentation window lost `" .. part .. "`:\n" .. twice.documentation.value
  )
end
io.write("nvim-check: the server's reply carries the block and the signature\n")
vim.cmd("qa!")
LUA

# No client anywhere: `Source:complete` falls back to the buffer scan, whose
# items say the same thing about the same declaration, and keep quiet about
# whatever the header they were read from did not spell out.
cat >"$check_dir/scanner-completion.lua" <<'LUA'
local project = vim.env.SLOPIUM_COMPLETION_PROJECT
vim.cmd("edit " .. project .. "/src/main.slp")
local bufnr = vim.api.nvim_get_current_buf()
assert(
  #vim.lsp.get_clients({ bufnr = bufnr }) == 0,
  "a language server is attached; the fallback cannot be checked like this"
)

local items = require("slopium.completion").items(bufnr)
local found = {}
for _, item in ipairs(items) do
  found[item.label] = item
end
local twice = assert(found.twice, "the scan did not offer `twice`")
assert(
  twice.detail == "fn twice(i64) -> i64",
  "the scan read the signature as `" .. tostring(twice.detail) .. "`"
)
assert(
  type(twice.documentation) == "table" and twice.documentation.kind == "markdown",
  "the scan sent no markdown documentation window"
)
for _, part in ipairs({ "Doubles a value by adding it to itself.", "fn twice(i64) -> i64" }) do
  assert(
    twice.documentation.value:find(part, 1, true),
    "the fallback lost `" .. part .. "`:\n" .. twice.documentation.value
  )
end
assert(found.main, "the scan did not offer `main`")
assert(found.main.detail == "fn main() -> i32", "zero parameters still read as a signature")
assert(
  found.main.documentation == nil,
  "the scan wrote prose above a declaration that had none"
)

-- A header left hanging open says nothing rather than guessing: the name is
-- still offered, wearing the phrase it always wore.
local unfinished = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_lines(unfinished, 0, -1, false, {
  ";; Left open, so no type can be said for it.",
  "(fn hang ((x i64",
})
local hang
for _, item in ipairs(require("slopium.completion").items(unfinished)) do
  if item.label == "hang" then
    hang = item
    break
  end
end
assert(hang, "the scan dropped a declaration it could not finish reading")
assert(
  hang.detail == "function in this buffer",
  "an unreadable header was described as `" .. tostring(hang.detail) .. "`"
)
assert(
  hang.documentation == nil,
  "the scan documented something it could not read"
)
io.write("nvim-check: the scanner alone carries the block and the signature\n")
vim.cmd("qa!")
LUA

SLOPIUM_LSP_SERVER="$server" SLOPIUM_COMPLETION_PROJECT="$check_dir/completion" \
  nvim --headless -u "$check_dir/init.lua" -S "$check_dir/server-completion.lua" \
  >/dev/null 2>"$check_dir/nvim.server.stderr" ||
  {
    cat "$check_dir/nvim.server.stderr" >&2
    fail "the language-server completion check failed"
  }
echo "nvim-check: the server's popup completes with the block and the signature ... ok"

SLOPIUM_COMPLETION_PROJECT="$check_dir/completion" \
  nvim --headless -u "$check_dir/init.lua" -S "$check_dir/scanner-completion.lua" \
  >/dev/null 2>"$check_dir/nvim.scanner.stderr" || {
    cat "$check_dir/nvim.scanner.stderr" >&2
    fail "the scanner-only completion check failed"
  }

echo "nvim-check: all editor plugin checks passed"

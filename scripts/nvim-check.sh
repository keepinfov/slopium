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
echo "nvim-check: all editor plugin checks passed"

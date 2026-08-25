# slopium.nvim

Локальная Neovim-поддержка языка:

- filetype `slopium` для `*.slp`;
- раздельная подсветка форм, функций, типов, параметров, bindings, полей и
  enum constructors;
- собственный `indentexpr` вместо встроенного Lisp-отступа: тот считает `(` и
  `)` в сишных исходниках самого Vim и научить его закрывающему `|)` нельзя
  (`D-151`), поэтому отступ считает `lua/slopium/indent.lua` — по тем же
  `lispwords`, которые задаёт `ftplugin`;
- snippets, builtins, функции, параметры и bindings текущего buffer для
  `nvim-cmp`; при подключённом `slopium-lsp` собственный источник отдаёт
  только snippets — остальное приходит от сервера;
- `omnifunc` fallback без `nvim-cmp`;
- semantic tokens, scoped completion, hover, definition, references и rename
  через `slopium-lsp`;
- асинхронные diagnostics через `slopic` после сохранения, если LSP недоступен;
- buffer-команда `:SlopiumCheck`.

Установка из корня репозитория:

```sh
./scripts/install-nvim.sh
```

Для обычного Neovim скрипт создаёт симлинку в
`$XDG_DATA_HOME/nvim/site/pack/slopium/start/slopium.nvim` (либо в
`~/.local/share/...`). Если найден lazy.nvim, он также создаёт маленькую
spec-симлинку `lua/plugins/slopium.lua`: это нужно конфигурациям, которые
отключают native packages. Существующие Lua-файлы и lockfile не меняются.

Плагин ищет `slopium-lsp` рядом с `slopic`, в `PATH`, затем в локальных
`target/debug` и `target/release`. Если сервер не найден, остаются regexp
подсветка, CMP/omnifunc и проверка сохранённого файла через `slopic`. Явные
пути:

```lua
vim.g.slopium_compiler = "/path/to/slopic"
vim.g.slopium_lsp = "/path/to/slopium-lsp"
```

Удаление:

```sh
./scripts/uninstall-nvim.sh
```

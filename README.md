# Slopium

Slopium (`Sl`) — небольшой статически типизированный язык с
S-expression-синтаксисом, ownership и нативной ahead-of-time компиляцией.
LLVM, виртуальная машина и интерпретатор не нужны.

Проект состоит из трёх программ:

- `slopic` — AOT-компилятор файла или полного source tree, аналог `rustc`;
- `slopium` — менеджер проектов, профилей, кэша и тестов, аналог Cargo.
- `slopium-lsp` — лёгкий language server поверх API компилятора.

Поддерживаемая платформа v0.2: `x86_64-unknown-linux-gnu`. Компилятор сам
генерирует x86-64 assembly, после чего вызывает `cc` только как assembler и
linker.

## Самый быстрый запуск на NixOS/Nix

Ничего устанавливать глобально не требуется.

```sh
nix shell .
slopium --version
slopic --version
```

`nix shell .` собирает toolchain и открывает shell, в котором доступны
`slopium`, `slopic` и подходящий Nix `cc`.

Создание и запуск проекта:

```sh
nix shell .
slopium new hello
cd hello
slopium run
slopium test
```

Другие flake-интерфейсы:

```sh
nix run .#slopium -- --help
nix run .#slopic -- --info
nix develop
nix flake check
```

`nix develop` предназначен для разработки самого компилятора и включает
Rust/Cargo, rustfmt, Clippy, GCC, binutils и GDB.

## Сборка без Nix

Требования:

- актуальный Rust toolchain с Cargo;
- GNU-compatible `cc` с libc development files;
- Linux x86-64 с glibc.

```sh
cargo build --workspace
```

Бинарники появятся здесь:

```text
target/debug/slopic
target/debug/slopium
```

Полная проверка репозитория:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
scripts/project-tests.sh
```

`project-tests.sh` прогоняет самостоятельные mini-project fixtures через
настоящий `slopium`: форматирование, проверку, dev/release-сборку, запуск,
языковые тесты, ожидаемые compile errors и runtime failures. Матрица покрытия:
[`tests/projects/README.md`](tests/projects/README.md).

Для локальной установки оба бинарника должны попасть в один `bin`-каталог:

```sh
cargo install --path crates/slopic --root "$HOME/.local" --locked
cargo install --path crates/slopium --root "$HOME/.local" --locked
cargo install --path crates/slopium-lsp --root "$HOME/.local" --locked
```

Убедитесь, что `$HOME/.local/bin` находится в `PATH`. Runtime source встроен в
toolchain и отдельно устанавливать его не нужно.

## Прямое использование `slopic`

Общая форма:

```text
slopic <INPUT> [--emit check|hir|mir|asm|obj|exe] [OPTIONS]
```

Если toolchain собран через Cargo, замените `slopic` на
`target/debug/slopic`.

Только проверить типы и ownership:

```sh
slopic examples/ownership.slp --emit check
```

Посмотреть промежуточные представления:

```sh
slopic examples/structs.slp --emit hir -o /tmp/structs.hir.json
slopic examples/structs.slp --emit mir -o /tmp/structs.mir.json
slopic examples/structs.slp --emit mir-text
```

`--emit mir-text` печатает MIR в читаемом виде: локальные переменные с типами,
базовые блоки, терминаторы и позиция в исходнике для каждой инструкции. HIR и
MIR — отладочный вывод без гарантий совместимости; их формат может меняться
между версиями.

Получить assembly или object-файл:

```sh
slopic examples/fibonacci.slp --emit asm -o /tmp/fibonacci.s
slopic examples/fibonacci.slp --emit obj -o /tmp/fibonacci.o
```

Собрать и запустить самостоятельный ELF:

```sh
slopic examples/fibonacci.slp --emit exe -o /tmp/fibonacci
/tmp/fibonacci
```

`slopic` автоматически материализует встроенный C runtime рядом с временным
assembly и удаляет временные файлы после линковки. Другую реализацию runtime
можно передать явно:

```sh
slopic program.slp --emit exe --runtime /path/to/slop_rt.c -o program
```

Запустить объявления `(test ...)` вместо обычного `main`:

```sh
slopic examples/match.slp --emit exe --test -o /tmp/match-tests
/tmp/match-tests
```

Полезные параметры:

```text
--profile dev|release       release включает constant folding
--target <triple>           сейчас: x86_64-unknown-linux-gnu
--cc <command>              assembler/linker driver
--diagnostic-format json    JSON diagnostics для IDE/CI
--source-root <directory>   собрать все .slp модули дерева
--dependency NAME=ROOT      добавить namespaced path dependency
--toolchain-dependency std  использовать встроенную std
--info                      protocol version и targets
```

`slopic` не читает `Slopium.toml`, не ищет проект и не управляет кэшем.

## Использование `slopium`

### Новый проект

```sh
slopium new hello
cd hello
```

Или создать проект в явно заданном месте:

```sh
slopium new hello --path /tmp/hello
```

Структура:

```text
hello/
├── Slopium.toml
├── .gitignore
└── src/
    └── main.slp
```

### Основные команды

```sh
slopium check                 # parser, types, ownership; без линковки
slopium build                 # dev ELF
slopium build --release       # release ELF
slopium run                   # собрать и запустить
slopium run -- --arg value    # аргументы программе
slopium test                  # собрать и выполнить (test ...)
slopium fmt                   # отформатировать все source-файлы
slopium fmt --check           # проверить форматирование без записи
slopium clean                 # удалить target/
slopium targets               # доступные targets
slopium compiler              # handshake с найденным slopic
```

Команды ищут `Slopium.toml` в текущем каталоге и его родителях. Можно указать
manifest явно:

```sh
slopium --manifest-path /path/to/project/Slopium.toml build --release
```

Артефакты находятся в:

```text
target/<target>/<dev|release>/<package-name>
target/<target>/<dev|release>/<package-name>-tests
```

Каждый модуль компилируется в отдельный object. Изменение только тела
пересобирает его object; изменение публичного интерфейса инвалидирует
потребителей. Итоговая линковка и cache key также учитывают manifests,
dependencies, target, profile, версию `slopic`, runtime и `cc`.

### `Slopium.toml`

```toml
[package]
name = "hello"
version = "0.1.0"
source = "src"
entry = "src/main.slp"

[dependencies]
std = { toolchain = true }
# geometry = { path = "../geometry" }

[build]
target = "x86_64-unknown-linux-gnu"

[profile.dev]
opt-level = 0
debug = true

[profile.release]
opt-level = 1
debug = false
```

`source` задаёт корень path-derived модулей. Path dependencies получают
namespace из ключа таблицы; `std` можно заменить path-пакетом с секцией
`[language-items]`. Поля profiles участвуют в cache key; release также включает
оптимизирующий MIR pass.

### Выбор compiler и `cc`

`slopium` ищет `slopic` рядом со своим executable и проверяет protocol version.
Другой компилятор задаётся переменной:

```sh
export SLOPIC=/path/to/slopic
```

Target выбирается в порядке:

1. `--target`;
2. `SLOPIUM_TARGET`;
3. `[build].target`;
4. встроенный host target.

`cc` выбирается в порядке:

1. `--cc`;
2. `SLOPIUM_CC_X86_64_UNKNOWN_LINUX_GNU`;
3. target config;
4. общий toolchain config;
5. `cc` из `PATH`.

`.slopium/config.toml`:

```toml
[toolchain]
cc = "cc"

[target.x86_64-unknown-linux-gnu]
cc = "x86_64-unknown-linux-gnu-gcc"
```

## Минимальный пример языка

```lisp
(fn fib ((n i64)) -> i64
  (if (< n 2)
      n
      (+ (fib (- n 1)) (fib (- n 2)))))

(fn main () -> i32
  (let message "fib(10)")
  (println (& message))
  (println (fib 10))
  0)

(test "fibonacci"
  (= (fib 10) 55))
```

Дополнительные программы находятся в [`examples/`](examples/):

- `fibonacci.slp` — функции и рекурсия;
- `ownership.slp` — move, borrow и clone;
- `lists.slp` — homogeneous lists;
- `structs.slp` — структуры;
- `enums.slp` — enum payload и exhaustive match;
- `match.slp` — boolean/integer match;
- `modules-demo/` — multi-file modules, generics, bundled `std` и owned list;
- `ctf-license-check/` — готовая reverse CTF-задача с TCP/Docker deployment.

Полное описание синтаксиса: [`docs/language.md`](docs/language.md).
Архитектура компилятора: [`docs/architecture.md`](docs/architecture.md).
Контракт diagnostics: [`docs/diagnostics.md`](docs/diagnostics.md).
Модель безопасности: [`docs/security.md`](docs/security.md).

## Neovim

В репозитории есть локальный плагин с раздельной подсветкой форм, функций,
типов, параметров, bindings, полей и enum constructors, Lisp-aware отступами,
semantic completion/navigation через `slopium-lsp`, дополнением через
`nvim-cmp`, fallback `omnifunc` и diagnostics от `slopic` после сохранения,
если LSP недоступен.

```sh
./scripts/install-nvim.sh
nvim examples/fibonacci.slp
```

Плагин устанавливается локальными симлинками в стандартный Neovim `site/pack`
и, если обнаружен lazy.nvim, в его каталог specs. Править существующий
`init.lua` или загружать сетевую зависимость не требуется.
Плагин автоматически запускает `slopium-lsp`, найденный рядом с toolchain или
в `PATH`. Для полной поддержки достаточно запустить Neovim из `nix shell .`,
установить toolchain или один раз выполнить `cargo build --workspace`.

```vim
:SlopiumCheck
```

Удаление: `./scripts/uninstall-nvim.sh`. Подробности:
[`editors/nvim/README.md`](editors/nvim/README.md).

## Ограничения

- только Linux x86-64 glibc;
- нет debug info: `.loc` и DWARF line tables ещё не эмитируются;
- register allocator не разрезает live interval, поэтому значение держит
  регистр и на тех участках, где ни разу не упоминается;
- ссылки и borrowed slices нельзя возвращать или хранить в aggregates и
  коллекциях;
- нет traits, bounds, registry/Git dependencies, stable FFI и lockfile;
- dependency graph поддерживает path и bundled-toolchain источники, но пока не
  registry resolution;
- единственный backend — прямой System V x86-64 codegen.

Это намеренно небольшой, но сквозной AOT compiler. Новые возможности
добавляются после стабилизации соответствующих compiler/runtime interfaces.

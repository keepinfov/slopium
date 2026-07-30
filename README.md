# Slopium

Slopium (`Sl`) — небольшой статически типизированный язык с
S-expression-синтаксисом, ownership и нативной ahead-of-time компиляцией.
LLVM, виртуальная машина и интерпретатор не нужны.

Проект состоит из трёх программ:

- `slopic` — AOT-компилятор файла или полного source tree, аналог `rustc`;
- `slopium` — менеджер проектов, профилей, кэша и тестов, аналог Cargo.
- `slopium-lsp` — лёгкий language server поверх API компилятора.

Поддерживаемые платформы: `x86_64-unknown-linux-gnu` и
`aarch64-unknown-linux-gnu`. Компилятор сам генерирует assembly для выбранной
архитектуры, после чего вызывает `cc` только как assembler и linker.

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
Rust/Cargo, rustfmt, Clippy, GCC, binutils, GDB, aarch64 cross-toolchain и
`qemu-aarch64`. Два последних нужны, чтобы собирать и запускать код второго
backend на x86-64-хосте.

## Установка в конфигурацию NixOS

Flake отдаёт overlay и два модуля, поэтому toolchain ставится системно, а не
через `nix shell` в каждой сессии.

```nix
{
  inputs.slopium = {
    url = "git+file:///home/x/dev/slopium";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  # в списке модулей хоста:
  #   inputs.slopium.nixosModules.default
}
```

```nix
# NixOS: slopic, slopium, slopium-lsp и completions для bash/zsh/fish
programs.slopium.enable = true;

# home-manager: плагин Neovim без симлинков вручную
programs.slopium.neovim.enable = true;
programs.slopium.neovim.lazySpec = true;  # если plugin manager сбрасывает rtp
```

- `nixosModules.default` — `programs.slopium.{enable,package,overlay}`; кладёт
  toolchain в `environment.systemPackages` и по умолчанию подключает overlay;
- `homeModules.default` — `programs.slopium.{enable,package,neovim}`; ставит
  плагин как native package в `~/.local/share/nvim/site/pack`;
- `overlays.default` — `pkgs.slopium` и `pkgs.vimPlugins.slopium-nvim`.

`lazySpec` нужен конфигурациям вроде lazy.nvim, которые по умолчанию сбрасывают
`runtimepath` и не видят native packages; он пишет
`~/.config/nvim/lua/plugins/slopium.lua`, указывающий на копию плагина в store.

Completions можно получить и без модуля:

```sh
slopium completions fish > ~/.config/fish/completions/slopium.fish
slopic --completions fish > ~/.config/fish/completions/slopic.fish
```

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
scripts/debug-check.sh
scripts/cross-check.sh
scripts/object-check.sh
```

`project-tests.sh` прогоняет самостоятельные mini-project fixtures через
настоящий `slopium`: форматирование, проверку, dev/release-сборку, запуск,
языковые тесты, ожидаемые compile errors и runtime failures. Матрица покрытия:
[`tests/projects/README.md`](tests/projects/README.md).

`debug-check.sh` проверяет DWARF line tables: сборку с `--debug`, отсутствие
debug-секций без него и реальную сессию GDB с breakpoint, шагом и backtrace.
Сессия пропускается, если GDB недоступен; в `nix develop` он есть.

`cross-check.sh` проверяет согласованность двух backend: собирает весь корпус
под обе архитектуры, запускает aarch64-сборки под `qemu-aarch64` и требует
совпадения stdout и кода выхода, включая программы, которые паникуют. Отдельно
проверяется ABI: функции Slopium линкуются с C-вызывающим кодом, собранным
настоящим toolchain, с числом аргументов больше, чем помещается в регистры.
Пропускается без cross-toolchain или qemu; в `nix develop` они есть.

`object-check.sh` проверяет собственный object writer компилятора против
системного ассемблера: тот же корпус собирается обоими путями под обе
архитектуры. На AArch64 сравнение побайтовое, на x86-64 — по инструкциям
(компилятор всегда берёт 32-битное смещение перехода, ассемблер укорачивает
те, что помещаются). На обеих сравниваются релокации и таблицы символов, и обе
версии линкуются и запускаются.

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
--debug                     DWARF line tables для отладки
--target <triple>           x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu
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

Библиотека создаётся с `--lib`: точка входа — `src/lib.slp`, `main` не нужен.

```sh
slopium new geometry --lib
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

В workspace каждая из этих команд принимает `-p <имя>` (один пакет) или
`--workspace` (все). Без флагов берётся пакет, в каталоге которого выполнена
команда.

Команды ищут `Slopium.toml` в текущем каталоге и его родителях. Можно указать
manifest явно:

```sh
slopium --manifest-path /path/to/project/Slopium.toml build --release
```

Артефакты находятся в:

```text
target/<target>/<dev|release>/<package-name>
target/<target>/<dev|release>/<package-name>-tests
target/<target>/<dev|release>/objects/<package-name>/
```

`target/` лежит в корне workspace; для одиночного пакета это его собственный
каталог.

У библиотеки (`entry` — `lib.slp`) нечего линковать, поэтому `build` для неё
означает `check`, а `run` — ошибка. `test` работает: harness сам приносит точку
входа.

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
# geometry = { path = "../geometry", version = "^1.2" }

[build]
target = "x86_64-unknown-linux-gnu"

[profile.dev]
opt-level = 0
debug = true

[profile.release]
opt-level = 1
debug = false
```

`source` задаёт корень path-derived модулей.

**Ключ в `[dependencies]` — это имя пакета.** Он же становится namespace, под
которым видны модули зависимости: `geometry = { path = "../geometry" }` даёт
`geometry:lib`. Если пакет по указанному пути называется иначе, это ошибка.
Одна и та же зависимость, до которой можно добраться двумя путями, попадает в
граф ровно один раз — namespace определяется именем пакета, а не путём, которым
до него дошли. Транзитивные зависимости видны под своими собственными именами.

`version` необязателен для path-зависимостей и проверяется, а не выбирается:
источник предлагает ровно одну версию. Поддерживаются `^`, `~`, `=`, `>=`, `>`,
`<=`, `<` и перечисление через запятую; голая версия означает `^`. Две
несовместимые версии одного имени в одном графе — ошибка.

Стандартную библиотеку заменяет любая прямая зависимость с секцией
`[language-items]` — важна секция, а не имя пакета. Двух таких в одном графе
быть не может.

Поля profiles участвуют в cache key; release также включает оптимизирующий MIR
pass. `debug` управляет DWARF line tables; если поле не указано, оно включено
для `dev` и выключено для `release`.

### `Slopium.lock`

`check`, `build`, `run` и `test` записывают рядом с manifest файл
`Slopium.lock` — разрешённый граф: имя, версия, источник и рёбра каждого
пакета, отсортированные по имени. Пути записываются относительно самого lock,
поэтому копия проекта в другом каталоге даёт тот же файл.

```sh
slopium tree                  # разрешённый граф; повтор помечается (*)
slopium build --locked        # упасть, если lock пришлось бы изменить
slopium build --offline       # не ходить в сеть (пока нечему)
slopium build --frozen        # --locked и --offline вместе
```

Приложению lock стоит коммитить, библиотеке — нет. Checksums появятся вместе с
content-addressed store.

### Workspace

Несколько пакетов могут делить один `Slopium.lock`, один `target/` и одно
разрешение графа. Корневой manifest перечисляет участников; `[package]` в нём
необязателен — manifest без него описывает только workspace.

```toml
[workspace]
members = ["crates/*"]
exclude = ["crates/scratch"]

# То, что участник берёт через `version.workspace = true`.
[workspace.package]
version = "0.3.0"

# То, что участник берёт через `<имя> = { workspace = true }`.
[workspace.dependencies]
geometry = { path = "vendor/geometry" }
```

```toml
# crates/app/Slopium.toml
[package]
name = "app"
version.workspace = true
entry = "src/main.slp"

[dependencies]
geometry = { workspace = true }
```

В `members` понимается только завершающая `*` — она означает «каждый
подкаталог с manifest». Унаследованный `path` записан относительно корня
workspace, а не участника. Наследуется запись целиком: `{ workspace = true }`
рядом с собственным `path` или `version` — ошибка.

```sh
slopium check --workspace     # все участники
slopium build -p app          # один
slopium run                   # тот, в чьём каталоге вы находитесь
```

`Slopium.lock` и `target/` — только в корне workspace, поэтому участники делят
собранные зависимости, а общая зависимость попадает в lock один раз. Участник,
на который ссылается `path`-зависимость другого участника, разрешается как этот
участник, а не читается заново.

`slopium test` запускает тесты только того пакета, который собирается: тесты
зависимости принадлежат ей самой.

### Отладка в GDB

`slopium build` уже собирает отлаживаемый бинарник:

```sh
slopium build
gdb target/x86_64-unknown-linux-gnu/dev/hello
```

```text
(gdb) break src/main.slp:3
(gdb) run
(gdb) next
(gdb) backtrace
```

Breakpoint по `файл:строка`, пошаговое выполнение и backtrace с указанием файла
и строки для каждого кадра работают, в том числе через границы модулей.
Расположения переменных не эмитируются, поэтому `print` по имени недоступен, а
имена функций в кадрах отображаются как ELF-символы.

Для отдельного файла тот же результат даёт `slopic`:

```sh
slopic examples/fibonacci.slp --emit exe --debug -o /tmp/fibonacci
```

### Кросс-компиляция под AArch64

```sh
nix develop
slopium build --target aarch64-unknown-linux-gnu
qemu-aarch64 target/aarch64-unknown-linux-gnu/dev/hello
```

`nix develop` уже экспортирует `SLOPIUM_CC_AARCH64_UNKNOWN_LINUX_GNU`, поэтому
менеджер находит cross-`cc` сам. Вне dev shell его нужно задать явно:

```sh
export SLOPIUM_CC_AARCH64_UNKNOWN_LINUX_GNU=aarch64-linux-gnu-gcc
```

То же самое для отдельного файла:

```sh
slopic examples/fibonacci.slp --emit exe \
  --target aarch64-unknown-linux-gnu \
  --cc aarch64-linux-gnu-gcc -o /tmp/fibonacci
```

Оба backend порождают одинаковое поведение: это проверяется
`scripts/cross-check.sh` на всём корпусе, а не декларируется.

### Объектные файлы

`slopic` сам кодирует инструкции и пишет relocatable ELF: ассемблер для этого
больше не нужен. Линковка по-прежнему системная — она знает, где лежит C-runtime
и как платформа запускает процесс.

Ассемблер остаётся в двух случаях. С `--debug` line tables строятся из
директив `.file`/`.loc`, а object writer не пишет DWARF. И по требованию:

```sh
export SLOPIUM_OBJECT_WRITER=external
```

Это запасной путь на случай ошибки в кодировщике: он не требует другого
компилятора.

### Размер бинаря

Линкер оставляет только то, что программа использует. Runtime компилируется с
секцией на функцию и линкуется с `--gc-sections`, поэтому программа без списков
не тащит `sl_rt_list_*` — это убирает только недостижимый код, поэтому включено
всегда. Strip символов — уже выбор (он убирает имена, нужные отладчику), и это
флаг `slopic --strip`, а не решение компилятора. Тесты (`test`) в сборку без
`--test` теперь вообще не эмитятся, поэтому `sl_rt_test_result` в релизе нет. И
trap-строки (`"division by zero"`, `"integer overflow"`) кладутся только если до
них реально может дойти проверка: в программе без деления их нет. `slopic` и
`slopium` линкуют одним списком флагов, так что standalone- и package-бинарь
ужимаются одинаково: небольшая программа — с ~22 КБ до ~14 КБ.

`slopic` — это механизм, а не политика: у него есть `--optimize`, `--debug`,
`--strip`, `--panic-abort`, и никакого понятия «release». Политику держит
менеджер. Профиль в `Slopium.toml` задаёт `opt-level`, `debug`, `strip` и
`panic`, а `slopium` разворачивает их в флаги. `strip` по умолчанию —
противоположность `debug` (отлаживаемая сборка и та, что отдают наружу, — разные
намерения); всё это переопределяемо:

```toml
[profile.release]
opt-level = 1
debug = false
strip = true          # по умолчанию, можно выключить
panic = "message"     # "abort" — падать молча, без строк ошибок

[profile.dev]
opt-level = 0
debug = true
strip = false
panic = "message"
```

`panic = "abort"` убирает *сообщения*, но не проверки: трап по-прежнему
срабатывает (bounds, overflow, деление на ноль проверяются), просто вместо
`fprintf` — тихий `exit`, и в бинаре не остаётся ни строк ошибок, ни `fprintf`.
Проверки не убираются никогда — это разменяло бы пару байт на undefined
behaviour.

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
2. `SLOPIUM_CC_<TARGET>`, например `SLOPIUM_CC_AARCH64_UNKNOWN_LINUX_GNU`;
3. target config;
4. общий toolchain config;
5. `cc` по умолчанию для target.

Для не-host target значение по умолчанию — cross-driver с именем target
(`aarch64-unknown-linux-gnu-cc`), а не голый `cc`: host-компилятор молча собрал
бы объекты не той архитектуры.

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

Декларативная альтернатива этому скрипту — `programs.slopium.neovim.enable` из
`homeModules.default`; тогда плагин берётся из store, а не из рабочего дерева.
Одновременно держать оба способа не нужно: сначала
`./scripts/uninstall-nvim.sh`, потом rebuild.

## Ограничения

- только Linux glibc: x86-64 и AArch64;
- release-бинарь стрипается и лишён символов; для отладки нужна dev-сборка,
  которая символы и line tables сохраняет;
- вырезаются только недостижимые функции runtime; неиспользуемые функции самой
  программы линкер пока не выкидывает — для этого нужна секция на функцию и в
  своём объекте (следующий шаг);
- debug info ограничен line tables: breakpoint по `файл:строка`, пошаговое
  выполнение и backtrace работают, но описания расположения переменных нет,
  поэтому `print x` в GDB недоступен;
- имена функций в backtrace показываются в mangled-виде (`sl_fn_<hex>`),
  так как DWARF генерирует ассемблер по ELF-символам;
- register allocator не разрезает live interval, поэтому значение держит
  регистр и на тех участках, где ни разу не упоминается;
- ссылки и borrowed slices нельзя возвращать или хранить в aggregates и
  коллекциях;
- нет traits, bounds, registry/Git dependencies и stable FFI;
- dependency graph поддерживает path и bundled-toolchain источники; registry и
  Git появятся в v0.4.3–v0.4.4, вместе с ними — checksums в lock;
- workspaces ещё нет: один manifest — один пакет;
- два backend — прямой codegen для System V AMD64 и для AAPCS64; сборка под
  aarch64 требует cross-`cc`, а запуск на x86-64-хосте — `qemu-aarch64`;
- object writer не пишет DWARF, поэтому сборка с `--debug` идёт через
  системный ассемблер;
- переходы на x86-64 всегда кодируются 32-битным смещением, без relaxation:
  `.text` выходит примерно на 6% больше, чем у ассемблера, при том же коде;
- на AArch64 проверка переполнения ветвится на trampoline в конце `.text`, и
  условная ветка достаёт ±1 МиБ: `.text` больше этого не соберётся. Это ошибка
  ассемблера во время сборки, а не неверный код.

Это намеренно небольшой, но сквозной AOT compiler. Новые возможности
добавляются после стабилизации соответствующих compiler/runtime interfaces.

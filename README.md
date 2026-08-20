# Slopium

Slopium (`Sl`) — небольшой статически типизированный язык с
S-expression-синтаксисом, ownership и нативной ahead-of-time компиляцией.
LLVM, виртуальная машина и интерпретатор не нужны.

Проект состоит из трёх программ:

- `slopic` — AOT-компилятор файла или полного source tree, аналог `rustc`;
- `slopium` — менеджер проектов, профилей, кэша и тестов, аналог Cargo.
- `slopium-lsp` — лёгкий language server поверх API компилятора.

Поддерживаемые платформы: `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu` и `x86_64-unknown-none` — bare metal, без libc:
программа сама себя стартует и сама поставляет четыре хука рантайма.
Компилятор сам генерирует assembly для выбранной архитектуры, после чего
вызывает `cc` только как assembler и linker.

## Где что лежит

- [CHANGELOG.md](CHANGELOG.md) — что изменилось в каждом релизе;
- [CONTRIBUTING.md](CONTRIBUTING.md) — путь от клона до pull request;
- [AGENTS.md](AGENTS.md) — контракт репозитория: ветки, коммиты, релизы и то,
  что обязано меняться вместе;
- [docs/](docs) — архитектура компилятора, язык, диагностика, упаковка и модель
  безопасности;
- [docs/decisions.md](docs/decisions.md) — журнал решений: почему всё устроено
  именно так, запись за записью.

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
    url = "github:keepinfov/slopium";
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
assembly и удаляет временные файлы после линковки. Runtime состоит из двух
единиц трансляции — `slop_rt_core.c` и `slop_rt_hosted.c` (`D-066`), — поэтому
`--runtime` повторяемый:

```sh
slopic program.slp --emit exe \
  --runtime /path/to/slop_rt_core.c \
  --runtime /path/to/slop_rt_hosted.c \
  -o program
```

Собрать без C-библиотеки под программой: только core-половина runtime, без
обёртки `main`, а библиотека по умолчанию — `core` вместо `std`. Четыре
символа — `sl_rt_alloc`, `sl_rt_free`, `sl_rt_abort`, `sl_rt_panic` —
предоставляет сама программа (`D-080`):

```sh
slopic program.slp --emit obj --freestanding --library -o program.o
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
--target <triple>           x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu,
                            x86_64-unknown-none
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

Если каталог оказался внутри workspace, который новый пакет не перечисляет,
`new` сам дописывает его в `[workspace] members` корневого манифеста и говорит
об этом. Без этого пакет был бы несобираем с первой секунды: пакет, лежащий
внутри workspace и не перечисленный в нём, отвергается при загрузке. Если
`members` уже достаёт до него шаблоном (`members = ["crates/*"]`), манифест не
трогается.

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
slopium package               # архив пакета + его digest
slopium vendor                # копии зависимостей в vendor/
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
# exclude = ["benchmarks", "**/*.png"]   # что не попадёт в архив
# include = ["src/**/*.slp"]             # или наоборот: только это
# c-sources = ["c/hal.c", "boot/start.s"] # C и assembly, что линкуются рядом

[dependencies]
std = { toolchain = true }
# geometry = "^1.2"                       # registry `default`
# geometry = { version = "^1.2", registry = "internal" }
# geometry = { path = "../geometry" }
# geometry = { path = "../geometry", version = "^1.2" }
# geometry = { git = "https://example.com/geometry.git" }
# geometry = { git = "https://example.com/geometry.git", tag = "v1.4.0" }

[build]
target = "x86_64-unknown-linux-gnu"
# linker-script = "link/kernel.ld"        # раскладка образа, только у корня

[profile.dev]
opt-level = 0
debug = true

[profile.release]
opt-level = 1
debug = false
```

`source` задаёт корень path-derived модулей. `entry` — модуль, с которого
начинается сборка. Отсутствие ключа — это и есть способ сказать, что пакет
библиотечный: он входится через `<source>/lib.slp`, то есть ровно то же самое,
что `entry = "src/lib.slp"`, только не написанное. Если такого файла нет,
сборка говорит, какой файл искала, — а не про поле, которое не заполнено.

**Ключ в `[dependencies]` — это имя пакета.** Он же становится namespace, под
которым видны модули зависимости: `geometry = { path = "../geometry" }` даёт
`geometry:lib`. Если пакет по указанному пути называется иначе, это ошибка.
Одна и та же зависимость, до которой можно добраться двумя путями, попадает в
граф ровно один раз — namespace определяется именем пакета, а не путём, которым
до него дошли. Транзитивные зависимости видны под своими собственными именами.

`version` необязателен для path- и git-зависимостей и проверяется, а не
выбирается: источник предлагает ровно одну версию. Поддерживаются `^`, `~`, `=`, `>=`, `>`,
`<=`, `<` и перечисление через запятую; голая версия означает `^`. Две
несовместимые версии одного имени в одном графе — ошибка.

Стандартную библиотеку заменяет любая прямая зависимость с секцией
`[language-items]` — важна секция, а не имя пакета. Двух таких в одном графе
быть не может.

Ключ, которого этот тулчейн не знает, — не ошибка: он печатается как
`warning[SL1200]` и игнорируется, потому что манифест едет вместе с пакетом и
его читает каждая версия тулчейна, которая до него доберётся. Иначе любое поле,
добавленное после 1.0, ломало бы все предыдущие. Ключ при этом называется
полным путём (`profile.dev.lto`), потому что второе, чем он может быть, — это
опечатка, а настройка, которую молча выбросили, хуже отвергнутой. В архив ключ
попадает как написан. Предупреждение поднимается для манифестов той рабочей
области, над которой идёт команда: манифест зависимости — дело зависимости.
Исключение — `.slopium/config.toml`: он принадлежит этому клону, никуда не
едет, поэтому незнакомый ключ там по-прежнему отвергается (`D-128`).

Поля profiles участвуют в cache key; release также включает оптимизирующий MIR
pass. `debug` управляет DWARF line tables; если поле не указано, оно включено
для `dev` и выключено для `release`.

### `Slopium.lock`

`check`, `build`, `run` и `test` записывают рядом с manifest файл
`Slopium.lock` — разрешённый граф: имя, версия, источник, контрольная сумма и
рёбра каждого пакета, отсортированные по имени. Пути записываются относительно
самого lock, поэтому копия проекта в другом каталоге даёт тот же файл.

`checksum` есть у пакета, чьи байты не могут измениться под lock: библиотека,
входящая в компилятор, пакет из git и опубликованная версия из registry. У
path-зависимости его нет — это рабочий каталог, и хеширование переписывало бы
lock на каждое нажатие клавиши.
Lock, который этот toolchain не может прочитать, перезаписывается с сообщением:
он целиком выводится из manifests. Под `--locked` это ошибка.

```sh
slopium tree                  # разрешённый граф; повтор помечается (*)
slopium tree --depth 1        # только прямые зависимости; срез помечается (...)
slopium tree --duplicates     # что используется больше чем одним пакетом
slopium build --locked        # упасть, если lock пришлось бы изменить
slopium build --offline       # lock, store, vendor и кеш индекса: без сети
slopium build --frozen        # --locked и --offline вместе
slopium update -p geometry    # единственная команда, двигающая lock
```

Приложению lock стоит коммитить, библиотеке — нет.

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

### Архив пакета и vendoring

```sh
slopium package               # target/package/<имя>-<версия>.sl.tar + digest
slopium vendor                # копии зависимостей в vendor/ + перенаправление
slopium vendor -p full        # только то, что нужно одному участнику workspace
slopium build --offline       # не тянуть ничего, чего ещё нет локально
```

Архив — обычный ustar tar, из которого убрано всё, что различается между
машинами: записи отсортированы, `mtime` и владельцы нулевые, права — `0644` и
`0755`, симлинков и устройств не бывает, всё лежит под одним каталогом
`<имя>-<версия>/`. Сжатия в формате нет: digest считается по tar, поэтому два
архива сравниваются через `cmp`, а сумма — через `sha256sum`. Две сборки в
разных каталогах и в разное время дают побайтово одинаковый файл.

В архив не попадают `target/`, `.git/`, `.slopium/`, каталог vendoring, а у
библиотеки — ещё и `Slopium.lock`. `exclude` добавляет к этому списку, `include`
задаёт содержимое целиком; вместе они — ошибка. В шаблонах `*` не переходит
через `/`, а `**` переходит.

`slopium vendor` копирует в `vendor/` каждую зависимость, которая не является
каталогом на этой машине, и пишет перенаправление в `.slopium/config.toml`.
С `-p` копируется только то, что нужно одному участнику workspace; перенаправление
при этом всё равно действует на весь workspace, поэтому команда называет тех
участников, которые после такой частичной копии перестанут собираться
`--offline`. Повторный `vendor` поверх перенаправления, написанного им же,
дописывает недостающие источники, а не отказывается: отказ остаётся для
`[source]`, написанных рукой.
Копии берутся из содержимо-адресуемого хранилища в `$SLOPIUM_HOME` (по умолчанию
`${XDG_CACHE_HOME:-~/.cache}/slopium`), где архив проверяется по digest **до**
распаковки. Перенаправление не меняет разрешение графа: пакет сохраняет имя,
версию, источник и запись в lock — меняется только то, откуда читаются байты,
поэтому `slopium check --locked` после vendoring проходит. Изменённая копия
ловится при каждой сборке (`SL1012`), и вернуть её на место может только
`slopium vendor`.

### Зависимости из git

```toml
[dependencies]
geometry = { git = "https://example.com/geometry.git" }               # ветка по умолчанию
geometry = { git = "https://example.com/geometry.git", branch = "main" }
geometry = { git = "https://example.com/geometry.git", tag = "v1.4.0" }
geometry = { git = "https://example.com/geometry.git", rev = "0f2c1a9" }
```

Скачиванием занимается сам `git`, запущенный как внешняя программа: он уже умеет
транспорты, ключи и credential helpers. Голый репозиторий лежит в
`$SLOPIUM_HOME/git/db/`, рабочей копии там нет, а конфигурация машины к нему не
подмешивается — правило `url.*.insteadOf` увело бы скачивание туда, чего lock не
называет. Пакетом становится `git archive` указанного коммита, приведённый к
тому же формату архива, что и опубликованный.

**Разрешение всегда фиксирует полный коммит**, и ветка записывается рядом с ним:
`source = "git+URL?branch=main#<40 hex>"`. Зафиксированная зависимость больше не
разрешается: ветка может двигаться сколько угодно, сборка останется на том
коммите, который записан в lock. `checksum` рядом позволяет проверить пакет, не
доверяя хешам самого git.

Submodules в v0.4 не скачиваются — дерево с `.gitmodules` отвергается при
разрешении (`SL1021`). Пакет из git не может объявлять `path`-зависимости: он
распаковывается в store, и записать такой путь в переносимый lock нечем.

После `slopium vendor` проект собирается с `--offline --locked` вообще без git и
без store — копии в `vendor/` проверяются по контрольным суммам из lock.

Разрешать новую зависимость offline тоже можно: индексные файлы, которые машина
уже скачивала, лежат в `$SLOPIUM_HOME/index/<digest адреса индекса>/`, и
`--offline` читает их. Кеш — только запасной путь: онлайновый запуск всегда
скачивает заново и всегда перезаписывает, а пакет, исчезнувший из индекса, из
кеша удаляется, чтобы offline никогда не противоречил последнему онлайновому
запуску. Registry-каталог (`file://` или относительный путь) читается напрямую и
в кеше не нуждается.

Подробности формата — в `docs/packaging.md`.

### Зависимости из registry

```toml
[dependencies]
geometry = "^1.2"                                     # registry `default`
physics = { version = "^2", registry = "internal" }
```

Registry — это каталог, который кто-то отдаёт по сети: `index/` с одной строкой
JSON на каждую опубликованную версию и `packages/` с архивами. Никакого сервера
для этого не нужно, и в этом репозитории его нет.

**Встроенного адреса registry в toolchain нет.** Адреса задаются на машине:

```toml
# .slopium/config.toml
[registry.default]
index = "https://packages.example.com"
```

Незаданный registry — ошибка (`SL1030`), а не скачивание непонятно откуда. В lock
записывается адрес индекса, а не его локальное имя, поэтому два разработчика,
называющие один индекс по-разному, получают один и тот же lock.

Registry — первый источник, у которого версий больше одной, поэтому выбор здесь
наконец что-то значит: берётся наибольшая подходящая версия, а если из-за неё
перестаёт разрешаться что-то другое — разрешение откатывается и берёт версию
постарше. Требования кандидатов читаются из индекса, но скачанный пакет
сверяется и с записью индекса (`SL1033`), и с опубликованным digest (`SL1034`):
индексу доверяют скорость, и больше ничего.

```sh
slopium add geometry@^1.2
slopium remove geometry
slopium update -p geometry --precise 1.3.0
slopium tree
```

`add` и `remove` правят `Slopium.toml` как текст и не переписывают то, чего не
трогали. `update` — единственная команда, двигающая lock; `-p` двигает ровно
один пакет, и это видно по diff'у самого lock.

### Подписи и публикация

```sh
slopium key new ~/.slopium/signing-key   # печатает публичную половину
slopium publish --key ~/.slopium/signing-key
slopium verify
```

`publish` — это `package` плюс подпись плюс три файла в каталоге: архив,
`.sig` рядом с ним и одна дописанная строка индекса. Публиковать можно только в
каталог: протокола загрузки нет, потому что нет сервера. Уже опубликованная
версия не переписывается (`SL1043`) — на неё может ссылаться чей-то lock.

Подписывается не digest, а утверждение «пакет с таким именем и такой версией
имеет такой digest», иначе подпись переносилась бы на другой пакет с теми же
байтами. Кому доверять — решает потребитель:

```toml
# .slopium/config.toml
[registry.default]
index = "https://packages.example.com"
trusted-keys = ["ed25519:1a2b…"]
```

Ключи заданы — пакет принимается только подписанный одним из них (`SL1040`,
`SL1041`, `SL1042`). Ключей нет — подписи не проверяются. Третьего состояния,
вроде «запомнить того, кто ответил первым», нет: это сделало бы первую загрузку
решением о доверии.

Сборка из lock под Nix — `lib.buildSlopiumPackage`: он читает `Slopium.lock`,
превращает каждую запись registry в fixed-output derivation с тем же checksum и
собирает `--offline --locked`. Разрешение при этом не выполняется вовсе, поэтому
Cargo и Nix собирают буквально один и тот же граф.

Подробности — в `docs/packaging.md` и `docs/security.md`.

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

### Bare metal: `x86_64-unknown-none`

Цель без libc под ней. От хостовой она отличается только окружением, а не
архитектурой: тот же backend, тот же ELF, но линковка идёт с
`-nostdlib -nostartfiles -static -no-pie`, из рантайма берётся только половина
`core`, и обёртки `main(argc, argv)` нет.

Поэтому программа обязана поставить два своих куска. Первый — четыре хука,
которых `slop_rt_core.c` не определяет: `sl_rt_alloc`, `sl_rt_free`,
`sl_rt_abort`, `sl_rt_panic`. Второй — точку входа `_start`, потому что звать её
теперь некому. И то и другое попадает в сборку через `[package] c-sources`,
который отдаёт `cc` и `.c`, и `.s`:

```toml
[package]
name = "bare"
entry = "src/main.slp"
c-sources = ["boot/start.s", "boot/hooks.c"]

[dependencies]
core = { toolchain = true }

[build]
target = "x86_64-unknown-none"
linker-script = "link/bare.ld"
```

`_start` вызывает вход программы по тому имени, под которым он линкуется. `main`
— единственная функция, сохраняющая имя без имени модуля, поэтому это всегда
`sl_fn_6d61696e`:

```asm
	.globl _start
_start:
	call	sl_fn_6d61696e
	movq	%rax, %rdi
	movl	$60, %eax
	syscall
```

`linker-script` необязателен: без него берётся стандартная раскладка. Путь
обязан лежать внутри пакета (`SL1101`), читается только у корневого пакета — и
попадает в кэш сборки по содержимому, так что правка скрипта вызывает
перелинковку.

Тестов у такой цели нет: harness — это сгенерированный `main`, который зовёт
`sl_rt_args_init` и `sl_rt_test_result`, а они есть только в hosted-половине
рантайма. `slopium test --target x86_64-unknown-none` поэтому отказывается, а не
собирает бинарник, который ничего не запускает.

Рабочий пример целиком — `tests/projects/freestanding/bare`.

### Ядро

`tests/projects/freestanding/kernel` — то же самое, доведённое до машины без
операционной системы. Загрузчик отдаёт управление в 32-битном защищённом
режиме, поэтому стартовый кусок обнуляет `.bss`, размечает первые 8 МиБ
страницами по 2 МиБ, включает long mode и только тогда зовёт `sl_fn_6d61696e`.
Всё, что выше этой границы, написано на Slopium.

Текстовый экран — это память, поэтому он берётся сырым указателем: адрес
становится `(Ptr u16)`, а ячейка пишется `volatile-write`. Байты сообщения потом
читаются обратно тем же указателем — тест сверяет то, что реально лежит в
видеопамяти, а не то, что туда собирались положить:

```lisp
(fn vga-write ((index u64) (byte u8)) -> unit
  (unsafe
    (let cells (as (Ptr u16) (vga-base)))
    (volatile-write (ptr-offset cells index) (vga-cell byte))))
```

Последовательный порт — не память: у него отдельное адресное пространство, и ни
один указатель его не называет. Поэтому `in` и `out` переходят границу C
обычными функциями, а драйвер UART над ними — инициализация, ожидание готовности
передатчика, побайтная выдача — пишется на Slopium:

```lisp
(extern "slop_outb" (outb (port u16) (value u8)) -> unit)
(extern "slop_inb" (inb (port u16)) -> u8)
```

Проверяет всё это `scripts/kernel-check.sh`: собирает ядро, перепаковывает образ
в 32-битный ELF (multiboot-загрузчик QEMU 64-битный не принимает), поднимает его
в `qemu-system-x86_64` и сверяет пришедшее по COM1 с `expected.serial`.

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
(take std:io println println-i64)

(fn fib ((n i64)) -> i64
  (if (< n 2)
      n
      (+ (fib (- n 1)) (fib (- n 2)))))

(fn main () -> i32
  (let message "fib(10)")
  (println (& message))
  (println-i64 (fib 10))
  0)

(test "fibonacci"
  (= (fib 10) 55))
```

Скалярные типы: `unit`, `bool`, `f64` и восемь целых — `i8`, `i16`, `i32`,
`i64`, `u8`, `u16`, `u32`, `u64`. Неявных преобразований нет: `(as T value)`
переводит любое целое в любое другое, расширяя по знаку источника и обрезая по
ширине цели, а всё, что касается `f64` и `bool`, отвергается по имени. Каждый
тип переполняется на своей границе и ловушка срабатывает вместо wraparound:
`u8` — выше `255`, `i8` — выше `127`. Шестнадцатеричный литерал — это набор
битов, а десятичный — число, поэтому `0xFF` есть и `u8` (`255`), и `i8` (`-1`),
а десятичное `255` — только `u8`.

Имя литерала объявляется на уровне модуля и подставляется в месте
использования; тип пишется после значения через `:`, если литерал сам его не
выбирает. Так же пишется и тип у `let` — аннотация относится к значению, а не к
имени, и нужна там, где выводить не из чего: у пустого контейнера тип элемента
не встречается ни в одном аргументе. Имя можно связать `let` дважды в одной
области: второй `let` — это новое имя, а не присваивание, и первое значение
роняется в конце области, как и всякое другое. `loop` — выражение: значение
уходит через `(break value)`, а `while` так не может, потому что заканчивается
по условию, где отдавать нечего. Ветка `match` может нести условие `when` между
образцом и телом; оно проверяется после того, как образец совпал, и до того,
как ветка взята, поэтому ветка с условием ничего не доказывает про полноту, а
вынести значение из имени внутри условия нельзя — следующая ветка сопоставляет
то же самое (`D-121`).

Тело берёт столько выражений, сколько нужно: ветка `match` и ветка `else` у
`if` читают их подряд и отвечают последним, а `then` остаётся одним выражением
— у `if`, где `else` обязателен, второй границы искать негде, и это ровно та
форма, которой пишется функция, отвечающая рано: короткий ответ сверху, работа
снизу. Односторонний условный оператор — это `when`: тело выполняется, когда
условие верно, значение — `unit` в любом случае, и `()`, стоявшее на месте
ветки, которой нет, больше не пишется. Слово то же, что и у условия ветки
`match`, и это решено намеренно, а не совпало (`D-127`).

`match` работает не только по агрегатам: целое, байт из строки, `bool` — всё,
что может назвать литеральный образец, сопоставляется так же, а лестница из
`if`, сравнивающих одно значение с набором констант, — это `match`, записанный
длинным способом. Последняя ветка связывает имя: у целого слишком много
значений, чтобы перечислить их все, поэтому одна ветка обязана отвечать за
остальные.

```lisp
(fn escaped ((byte i64)) -> String
  (match byte
    (34 "\\\"") (92 "\\\\") (10 "\\n") (13 "\\r") (9 "\\t")
    (other (if (< other 32) (unicode-escape other) ""))))

(const com1-data 0x3F8 : u16)

(fn describe ((reading (& Reading))) -> i64
  (match reading
    ((Reading:Retry attempt) when (> (clone attempt) 3) 0)
    ((Reading:Retry attempt) (clone attempt))
    ((Reading:Silent) (- 0 1))))

(fn doubled-at ((stop i64)) -> i64
  (let mut n 0)
  (loop
    (set n (+ n 1))
    (when (= n stop) (break (* n 2)))))

(fn scores () -> (Map String i64)
  (let mut table (map-new hash equals) : (Map String i64))
  table)
```

Объявление можно снабдить аннотацией: это список между словом объявления и его
именем, и таких списков может быть несколько. Слот заканчивается на имени —
атоме у `fn`, `struct`, `enum` и `const` и строке у `extern` и `test` — поэтому
ничего из того, что объявление умело писать раньше, не стало двусмысленным
(`D-122`). У `export` и `take` слота нет: они не вводят имени. Аннотаций две:
`inline` — подсказка оптимизатору, что тело стоит копировать в вызывающих, и
`deprecated` — предупреждение в каждом месте использования, с необязательным
сообщением. Ни то, ни другое слово нигде больше не занято: смысл у них есть
только в слоте, так что переменную можно назвать `inline`. Аннотация не на том
объявлении, неизвестное имя, лишний аргумент и повтор — отказ по имени.

```lisp
(fn (inline) blend ((a i64) (b i64)) -> i64 (* (+ a b) 2))

(fn (deprecated "вызывайте `parse-line`") parse ((s (& String))) -> i64 0)

(const (deprecated) retry-limit 3)
```

Использование `deprecated` — это предупреждение, а не ошибка: программа
собирается, а `slopic` печатает `warning[SL0800]` и выходит с нулём. Слот несут
все шесть форм объявления, в том числе те, к которым сегодня не подходит ни
одна аннотация: механизм и есть смысл этой версии — `repr` для чужой записи и
форма обработчика прерывания придут вместе с целью, которой они нужны
(`D-110`), а после заморозки новую форму добавить нельзя, новую аннотацию —
можно.

Изменить значение внутри структуры можно через исключительное заимствование:
`match` по `(&mut ...)` связывает каждое поле как `(&mut ...)` этого поля, а
такое имя — это место, которому `set` присваивает, роняя то, что там лежало
(`D-120`). Поэтому `map-insert` и `set-add` пишут в контейнер, а не пересобирают
его:

```lisp
(struct Counter ((count i64) (label String)))

(fn bump ((counter (&mut Counter))) -> unit
  (match counter
    ((Counter :count count :label label)
      (set count (+ (clone count) 1))
      (set label "bumped"))))
```

`set` пишет только в поле, которое разобрал сам `match`: ни имя из
разделяемого заимствования, ни параметр типа `(&mut T)` местом не являются.
Прочитать поле — это `clone`, как и через `(& ...)`; вынести значение из него
нельзя, потому что это заимствование, а не владение.

Регистр устройства — это байт, половина слова или слово по фиксированному
адресу, и добраться до него — единственное в языке, что компилятор не может
доказать безопасным. Поэтому это пишется словом: `(Ptr T)` — сырой указатель,
а `volatile-read`, `volatile-write`, `ptr-offset` и преобразование `as` в
указатель и обратно живут внутри `(unsafe ...)`:

```lisp
(fn set-bits ((port (Ptr u8)) (mask u8)) -> unit
  (unsafe
    (volatile-write port (bit-or (volatile-read port) mask))))
```

Указывать можно только на скаляр, `ptr-offset` считает элементы, а не байты.
`unsafe` не отключает ни проверки границ, ни проверки переполнения (`D-031`) и
не наследуется телом `lambda`: он говорит ровно одно — компилятор перестал
доказывать, что адрес куда-то указывает. Подробнее — в
[`docs/security.md`](docs/security.md).

Заимствовать можно и безымянное значение, если это аргумент вызова:
`(println (& "hello"))` и `(println (& (concat (& "task #") (& (from-i64 id)))))`
пишутся как написаны. Временное живёт до возврата из того вызова, которому его
передали, и там же освобождается; поэтому позиция аргумента — единственная, где
это разрешено: `(let text (& "x"))` отвергается, потому что освобождать было бы
негде, и диагностика предлагает дать значению имя.

Граница C закрыта списком, и он такой (`D-065`, `D-124`). Внутрь: любой целый
тип, `f64`, `bool`, `(Ptr T)`, `(& String)` как NUL-терминированный `const
char *` и `(& (Slice T))` как указатель и длина. Внутрь с правом записи:
`(&mut (List T))` и `(&mut (Array T N))` — указатель на элементы и их число,
которые C заполняет, но не переразмечает. Наружу: `(&mut i64)`, `(&mut u64)`,
`(&mut f64)` и `(&mut (Ptr T))` — out-параметр, слот которого целое машинное
слово, поэтому узкие целые и `bool` отвергаются поимённо. И `(Fn (…) …)` над
скалярами — указатель на функцию, в позиции аргумента обязан быть именем
верхнеуровневой `fn`: `lambda` не переходит, потому что это блок с окружением,
а указателю на функцию окружение носить негде. Возвращается `unit`, скаляр,
`(Ptr T)` или `String`, которой владеет вызывающая сторона. Агрегат по значению
не переходит ни в одну сторону.

```lisp
(extern "hal_fill" (hal-fill (into (&mut (List i64)))) -> unit)
(extern "hal_divmod" (hal-divmod (value i64) (by i64) (rest (&mut i64))) -> i64)
(extern "hal_apply" (hal-apply (step (Fn (i64) i64)) (value i64)) -> i64)
```

Ввод и вывод — это библиотека, а не язык: `std:io`, `std:string`, `std:process`
и `std:fs` написаны на Slopium поверх `extern` (`D-063`). Библиотека — два
пакета: `core` — то, что доступно программе без libc (`Option`, `Result`,
`string`), а `std` — это `core` плюс то, чему нужна операционная система.

`std:string` пишет число и в шестнадцатеричном виде: `hex-from-u64` — голые
цифры, `hex-prefixed-from-u64` — они же под `0x`. Ширина — это пол, а не
потолок: не хватает цифр — дополняются нулями, а значению, которому нужно
больше, ничего не обрезается; ноль означает естественную ширину. Буквы
заглавные — так этот язык пишет шестнадцатеричный литерал, поэтому
напечатанное можно вставить обратно в программу. Знаковое значение печатается
своим набором битов: `(hex-from-u64 (as u64 value) 16)` (`D-112`, `D-129`).

Упасть намеренно — это `std:panic` (в `core` он называется `core:panic`):
`(panic сообщение)`, `(assert условие сообщение)` и `(unreachable)`. Все три
завершают программу статусом 101 — тем же, которым её завершает переполнение
или выход за границу, — и печатают сообщение в stderr. Поймать это нельзя:
`Result` несёт отказ, на который вызывающая сторона может ответить (`D-087`), а
это другой случай. Каждая возвращает `unit`, потому что типа «никогда» в языке
нет, поэтому panic пишется там, где стоит оператор, а не там, где ждут значение
(`D-130`).

`std:test` — это то, что говорит падающий тест. `test` отвечает `bool`, и
harness печатает имя и вердикт, поэтому два разошедшихся значения иначе
теряются; `equal-i64`, `equal-u64` и `equal-text` сравнивают ровно как `=` и при
расхождении оставляют записку:

```text
test main:сумма ... FAILED: expected 42, got 41
```

Это не assert, и именно поэтому они разные функции: упавший assert завершает
программу, а набор тестов, останавливающийся на первом отказе, сообщает об
одной проблеме за прогон.

Ничто в ней не завершает программу за вас: конец ввода, нечисловая строка,
отсутствующий аргумент или переменная — это `None`, а отказ файловой операции —
`Err` с `errno` (`D-087`). Единственное, что завершает, — `std:panic`, который
для этого и написан и которого надо попросить.

Пакет объявляет `std = { toolchain = true }` в манифесте; одиночный файл получает библиотеку сам, `--no-std` её отключает,
а `--freestanding` даёт вместо неё `core`.

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
Формат пакета и хранилище: [`docs/packaging.md`](docs/packaging.md).
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
- целое любой ширины занимает машинное слово, поэтому `(List u8)` — это восемь
  байт на элемент; упаковка — отдельная задача с измерением перед ней;
- нет traits, bounds и stable FFI;
- dependency graph поддерживает path, bundled-toolchain, git и registry;
  подписи пакетов и `slopium publish` — v0.4.5;
- опубликованный пакет может зависеть только от своего registry и toolchain:
  ни `path`, ни `git`, ни чужой индекс (`SL1032`);
- workspace разрешает участников по отдельности, поэтому два участника с
  требованиями, выбирающими разные версии одного пакета, — ошибка, а не общий
  подбор;
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

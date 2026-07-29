# Slopium License Center

Небольшая CTF-задача категории reverse engineering. Участнику выдаются:

- `src/main.slp`;
- собранный ELF `slopium-license` (необязательно, но удобно);
- адрес сервиса вида `nc ctf.example.org 31337`.

Сервис просит `ticket id` и `activation pin`. При правильной паре он читает
`FLAG` из окружения процесса и печатает его. Настоящего флага в исходнике,
бинарнике и Docker-образе быть не должно.

## Локальная проверка без Docker

Из корня репозитория:

```sh
cargo build --workspace
target/debug/slopium \
  --manifest-path examples/ctf-license-check/Slopium.toml \
  build --release

printf '1337\n4242\n' |
  FLAG='slopium{local_test}' \
  examples/ctf-license-check/target/x86_64-unknown-linux-gnu/release/slopium-license
```

Ожидаемый конец вывода:

```text
license accepted
slopium{local_test}
```

## Запуск TCP-сервиса

Нужны Docker с Compose plugin. Из каталога задачи:

```sh
FLAG='slopium{real_secret_flag}' docker compose up --build -d
nc 127.0.0.1 31337
```

`compose.yaml` собирает toolchain и challenge в build-stage, а в финальный
контейнер переносит только ELF и `socat`. Контейнер работает без root,
capabilities и записи в root filesystem.

Для production лучше передавать `FLAG` из secret-хранилища оркестратора, а не
записывать его в Compose-файл, Dockerfile, CI log или image layer. Ограничение
числа соединений и sandbox каждого процесса должны выполняться внешней CTF
платформой.

## Материалы организатора

Разбор находится в `SOLUTION.md`. Не включайте его в public handout.

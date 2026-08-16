<!--
`AGENTS.md` is the contract this repository is held to; the checklist below is
its short form. Keep the lines that apply, delete the ones that do not, and say
plainly what you could not run rather than leaving a box ticked on faith.
-->

## What is now true that was not

<!-- One or two paragraphs, the same ones your commit body carries. -->

## Companions (AGENTS.md §8)

- [ ] Syntax, keyword, operator, or builtin — both backends, `docs/language.md`,
      `editors/nvim/syntax/slopium.vim`,
      `editors/nvim/lua/slopium/completion.lua`, the keyword lists in
      `crates/slopium-lsp/src/main.rs`, and a fixture under `tests/projects`.
- [ ] A new refusal — a stable `SL` code and a `compile_fail` case (`.slp`,
      `.expect.json`, `.stderr`).
- [ ] Diagnostic contract or code family — `docs/diagnostics.md`.
- [ ] Compiler stage, MIR, optimizer, or ownership — `docs/architecture.md`.
- [ ] Codegen or an encoder — `scripts/object-check.sh`, plus
      `scripts/cross-check.sh` for AArch64.
- [ ] C runtime — `scripts/runtime-check.sh`.
- [ ] Standard library — `scripts/core-check.sh` and the `std` fixtures.
- [ ] Manifests, resolution, lockfiles, registries, archives, or signatures —
      `docs/packaging.md`, the matching `scripts/*-check.sh`, and regenerated
      fixtures via `SLOPIUM_UPDATE_FIXTURES=1 scripts/publish-check.sh`.
- [ ] Trust, signing, verification, or offline behaviour — `docs/security.md`.
- [ ] A CLI flag or subcommand — clap, completions, `README.md`.
- [ ] Anything a user of the language sees — `README.md`.

## Validation

- [ ] `scripts/verify.sh` is green.

Anything you could not run, and why:

<!-- e.g. "no aarch64 toolchain here, so cross-check.sh did not run" -->

## Contract

- [ ] Version: bumped in this commit if it is a `feat`, untouched otherwise.
- [ ] Message: `type(scope): imperative lowercase`, at most 95 columns, body of
      one or two paragraphs wrapped at 80, no lists, no trailers, no AI
      attribution.
- [ ] Nothing here belongs to somebody else — no unrelated file, no `.notes/`,
      no build output.

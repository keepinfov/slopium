# Repository instructions

These instructions apply to the whole repository and to every agent or
contributor working in it, with or without the private `.notes/` directory. A
more specific `AGENTS.md` may add stricter rules for its own directory, but it
may not relax anything below.

## Precedence

When sources disagree, follow this order:

1. the user's latest direct instruction;
2. this file;
3. `.notes/` when it is present, in its own documented order (§2);
4. tracked documentation — `docs/`, `tests/README.md`, `editors/nvim/README.md`,
   `README.md`;
5. the code itself.

If a rule here blocks what you were asked to do, say so and ask. Do not resolve
the conflict silently.

## 1. What this repository is

Slopium is a small statically typed language with S-expression syntax, affine
ownership, and native ahead-of-time compilation. There is no LLVM, no VM, and no
interpreter under it: the compiler emits assembly and object code itself and
calls `cc` only as assembler and linker. Targets are
`x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`.

The Cargo workspace:

- `crates/slopic-core` — the compiler: lexer, S-expression parser, AST, package
  analysis, semantic analysis (types, generics, ownership, borrows),
  monomorphization, MIR lowering and verification, the release optimizer,
  register allocation, the x86-64 and AArch64 encoders, the ELF writer, and the
  diagnostic contract. It never discovers manifests and never touches the
  network — that boundary is the architecture, not a detail.
- `crates/slopic` — the compiler CLI. It is handed a source root and dependency
  roots explicitly.
- `crates/slopium` — the project manager: manifests, profiles, target selection,
  cache, build, run, test, publish.
- `crates/slopium-manifest` — manifests, lockfiles, resolution, registries,
  package archives, git sources, Ed25519 signatures, the store.
- `crates/slopium-lsp` — the language server over the compiler API.
- `crates/slopium-std` — support shared by the Rust-side crates.

Outside the workspace:

- `std/core`, `std/std` — the standard library, written in Slopium;
- `runtime/slop_rt_core.c` (freestanding) and `runtime/slop_rt_hosted.c` — the C
  runtime;
- `editors/nvim` — the shipped Neovim plugin. This is project code, not the
  user's editor configuration;
- `scripts/` — the check suite; `scripts/verify.sh` runs all of it;
- `tests/projects` — end-to-end fixtures; `tests/registry` and `tests/consumer`
  — a published registry and its consumer, generated and committed (see
  `tests/README.md`);
- `docs/` — `architecture.md`, `language.md`, `diagnostics.md`, `packaging.md`,
  `security.md`. English;
- `README.md` — the user-facing guide. Russian, deliberately.

Everything else — code, comments, diagnostics, documentation under `docs/`,
commit messages — is written in English.

## 2. Planning notes

Project planning lives in `.notes/`, which is gitignored and excluded from the
Nix source filter and from release archives.

If `.notes/` is present, before changing anything:

1. read `.notes/STATUS.md`, `.notes/DECISIONS.md`, and `.notes/ROADMAP.md`;
2. read the active file under `.notes/plans/` for the task;
3. do not implement a design recorded as `proposed`, `deferred`, or `rejected`.

While working there: update `STATUS.md` when the active state changes, mark
acceptance criteria in the active plan, record accepted or rejected design
choices in `DECISIONS.md`, add a dated file under `handoffs/` after substantial
work rather than rewriting an old one, and put directed coordination under
`messages/`.

If `.notes/` is absent — a fresh public clone, CI, a worktree — work from the
tracked documentation and the user's instruction. Do not recreate private
project history from guesses, do not invent a decision log, and do not implement
a design you cannot find authority for in tracked docs or the user's words. Ask
instead.

Never `git add -f .notes`, never copy note contents into tracked files, and
never store credentials, flags, tokens, or private registry URLs there. A commit
body may *cite* a decision identifier such as `D-106`, because that is how this
history refers to its own reasoning, but the message must stand on its own for a
reader who has no notes.

## 3. Non-negotiable constraints

- Preserve unrelated work. Do not stash, reset, rewrite, absorb, reformat, or
  clean anything outside the current task. If the working tree already holds
  changes you did not make, stop and ask before committing.
- Do not run destructive Git or filesystem commands without explicit approval.
- Do not mutate a remote — push, tag push, pull request, release — unless the
  user authorizes that exact action. `main` here is routinely many commits ahead
  of `origin/main`; that is normal and is not an invitation to push.
- Never force-push and never rewrite published history.
- No secrets in source, logs, commits, or responses. The Ed25519 key written
  into `scripts/publish-check.sh` is a deliberate public fixture: it signs two
  committed test files and nothing real is ever published under it.
- Never add an AI or agent attribution trailer — no `Co-authored-by`,
  `Generated-by`, `Assisted-by`, or equivalent. Not once, not even when a
  default tells you to.
- Do not add an external dependency without approval. The current surface is
  small and intentional: `clap`, `clap_complete`, `serde`, `serde_json`, `toml`,
  `ed25519-dalek`, and the three LSP crates. `slopic-core` depends on `serde`,
  `serde_json`, and `slopium-std`, and nothing else.
- The workspace contains zero `unsafe`. Introducing any requires explicit
  approval and a written justification.
- Do not weaken, delete, or rewrite a test, a fixture, or an expectation file to
  make a check pass. If an expectation is wrong, say why before changing it.
- Do not edit `Cargo.lock` by hand or bump a dependency unless the task is about
  that dependency.

## 4. Task flow

Work directly in the working tree for a single focused change. For anything that
touches three or more of {compiler, manager, manifest layer, runtime, standard
library, editor plugin}, or that combines a format change with a behavior
change, present a plan first: goals, the order of the work, and what "done"
means. Use a local branch when the work will take several attempts; delete it
after landing.

Whatever the route, one unit of work lands as **exactly one commit on `main`**.
WIP and fixup commits are fine while working and must not survive. If `main`
moved underneath you, stop and ask rather than rebasing or transplanting on your
own.

## 5. Commit contract

This is the part a contributor cannot guess from the code, so it is spelled out
completely. Read `git log` for calibration; the last twenty commits are all in
this form.

### Subject

```text
type(scope): imperative lowercase description
```

- Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `build`, `ci`,
  `chore`, `revert`.
- Scopes in use: `slopium` (the language and the toolchain — the default for
  compiler, manager, standard library, and runtime work), `manifest`, `docs`.
  Ask before inventing a new one.
- English, imperative, lowercase after the colon, no trailing period.
- Aim for 72 characters, hard limit 95.
- Say what the language or the toolchain now does, not which files moved. The
  house form is two clauses naming the two things that changed:

  ```text
  feat(slopium): read what a borrow points at, and match without taking it apart
  feat(slopium): print a float exactly, without a C library under it
  fix(slopium): resolve a shared dependency under every namespace
  ```

- Never `feat(slopium): update sema.rs`, never `various fixes`, never a version
  number in the subject.

### Body

- One or two paragraphs. Two is the maximum, and each is a paragraph of prose,
  not a disguised list.
- Wrap at 80 columns.
- Prose only: no bullet lists, no file inventory, no step-by-step recap of the
  session, no "this commit", no "we".
- Backtick every identifier, type, operator, diagnostic code, path, and decision
  id.
- The first paragraph says what is now true that was not true before, and why it
  was worth doing.
- The second, when there is one, says what the work uncovered — a pre-existing
  bug, a hole nothing had ever exercised, a consequence that fell out of the
  design. This is the paragraph that makes the history worth reading; write it
  when it is honest, omit it when there is nothing to report.
- No trailers. `Fixes #...` or `Refs #...` only for an issue the user gave you.
  `BREAKING CHANGE:` requires approval before the work starts, together with a
  `!` in the subject.

If the explanation does not fit in two paragraphs, it is not a longer commit
message. It is a `docs/` change when a user of the language needs it, and a
`.notes/DECISIONS.md` or handoff entry when it is project reasoning.

## 6. Version and tag

Every `feat` commit that lands is a release:

- bump `workspace.package.version` in `Cargo.toml` and let `Cargo.lock` follow,
  in the same commit as the work;
- pre-1.0 semantics: minor for a language or toolchain capability, patch for a
  behavior fix inside an existing capability;
- the tag is **lightweight**, named `vX.Y.Z`, placed on that commit.

A `docs`, `ci`, `chore`, or standalone `fix` commit does not bump the version
and is not tagged.

Creating the tag is a release action: propose the version and the rationale and
wait for approval. Pushing the commit or the tag is a remote mutation — confirm
the remote and the refspec first.

## 7. What must change together

A change is incomplete until its companions are updated. This table is the main
reason a contributor without notes can still land a correct commit.

| You changed | You must also touch |
| --- | --- |
| Syntax, a keyword, an operator, a builtin | both backends, `docs/language.md`, `editors/nvim/syntax/slopium.vim`, `editors/nvim/lua/slopium/completion.lua`, the keyword lists in `crates/slopium-lsp/src/main.rs`, a fixture under `tests/projects` |
| Anything the compiler refuses | a stable code in the right `SL` family and a `compile_fail` case — `.slp`, `.expect.json`, `.stderr` — under `crates/slopic-core/tests/compile_fail` |
| The diagnostic contract or a code family | `docs/diagnostics.md` |
| A compiler stage, MIR, the optimizer, or ownership rules | `docs/architecture.md` |
| Codegen or an instruction encoder | `scripts/object-check.sh`, and `scripts/cross-check.sh` when it is AArch64 |
| The C runtime | `scripts/runtime-check.sh`, which runs it under valgrind and ASan |
| The standard library | `scripts/core-check.sh` and the `std` fixtures under `tests/projects` |
| Manifests, resolution, lockfiles, registries, archives, signatures | `docs/packaging.md`, the matching `scripts/{package,registry,publish,git}-check.sh`, and regenerated fixtures |
| Anything trust-related — signing, verification, offline behavior | `docs/security.md` |
| A CLI flag or subcommand | the clap definition, generated completions, and `README.md` |
| Anything a user of the language sees | `README.md` (Russian) |

Regenerate the committed registry and consumer with:

```sh
SLOPIUM_UPDATE_FIXTURES=1 scripts/publish-check.sh
```

Their being byte-identical on a re-run is itself an assertion — reproducible
archives and deterministic Ed25519 signing. A diff there is a regression, not a
timestamp.

## 8. Validation

The gate for a code change is the full suite, from the repository root:

```sh
scripts/verify.sh
```

It runs `cargo fmt --all -- --check`, `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`, then
`project-tests.sh`, `package-check.sh`, `git-check.sh`, `registry-check.sh`,
`publish-check.sh`, `runtime-check.sh`, `core-check.sh`, `debug-check.sh`,
`cross-check.sh`, `object-check.sh`, and finally `nix flake check`.

Rules:

- A failing required check blocks the commit. If the failure is demonstrably
  pre-existing, show before-and-after evidence and ask.
- If you could not run part of the suite — no Nix, no AArch64 toolchain, no
  valgrind — say exactly which part and why in your report. A skipped check is
  never reported as a passing one. `project-tests.sh` skips the cross target on
  its own when the toolchain is absent; that is expected and it says so.
- For a docs-only change, a content and Markdown review plus `git diff --check`
  is enough.
- CI mirrors a subset: fmt, test, clippy, `runtime-check.sh` with
  `SLOPIUM_ASAN_DETECT_LEAKS=1`, `project-tests.sh`, and `nix flake check`.
  Passing locally is the standard; CI is the backstop.

## 9. Code rules

- Rust 2021, `rustfmt` defaults, clippy clean at `-D warnings`.
- Keep the `slopic` / `slopium` split: the compiler is handed roots, the manager
  finds them. A compiler that reads a manifest or opens a socket is a bug in the
  design, not a feature.
- Every refusal about something a person wrote — a form, a type, a manifest
  field, a dependency graph — gets a stable diagnostic code. A failure to *do*
  something, such as an I/O error the operating system already explained, does
  not.
- Diagnostics carry accurate spans. A new one without a span is not finished.
- Comments explain why, not what. Match the density and voice of the file you
  are in.
- Do not commit build output: `target/`, `*.o`, `*.s`, `*.out`, `nvim.log`, and
  lockfiles under `tests/projects` and `examples` are ignored for a reason.

## 10. Before you commit

- [ ] `scripts/verify.sh` is green, or the exact subset you ran and why is in
      your report.
- [ ] `git status` and `git diff --cached --name-only` reviewed — no `.notes`,
      no build output, no stray fixture.
- [ ] The companions in §7 are updated.
- [ ] The version is bumped if this is a `feat`, and untouched otherwise.
- [ ] Subject: conventional, imperative, lowercase, ≤ 95 characters, about
      behavior rather than files.
- [ ] Body: one or two paragraphs, wrapped at 80, no lists, no trailers, no AI
      attribution.
- [ ] Nothing pushed and nothing tagged unless the user asked for it.

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
4. tracked documentation — `docs/decisions.md` for why something is the way it
   is, then the rest of `docs/`, `tests/README.md`, `editors/nvim/README.md`,
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
- `scripts/` — the check suite; `scripts/verify.sh` runs all of it, and
  `scripts/release-check.sh` decides the version invariants of §7;
- `tests/projects` — end-to-end fixtures; `tests/registry` and `tests/consumer`
  — a published registry and its consumer, generated and committed (see
  `tests/README.md`);
- `docs/` — `decisions.md`, `architecture.md`, `language.md`, `diagnostics.md`,
  `packaging.md`, `security.md`. English, apart from `security.md`;
- `README.md` — the user-facing guide. Russian, deliberately;
- `CHANGELOG.md` — what each release changed, in the form §7 keeps it, and
  `changelog.d/` — where a change writes its entry until a release collects
  it;
- `CONTRIBUTING.md` — the short path from a clone to a reviewable pull request;
- `CLAUDE.md` — a symlink to this file, so a tool that looks only for that name
  still reads the contract.

Everything else — code, comments, diagnostics, documentation under `docs/`,
commit messages — is written in English.

## 2. Decisions and planning notes

**Decisions are tracked.** `docs/decisions.md` holds every design decision this
project has taken, numbered `D-001` upwards, each with its status, its date and
the reasoning that produced it. It is where a `D-042` in a commit body or a
document resolves, and it is read before a design question is reopened: a
decision recorded as `deferred` or `rejected` is not implemented without a new
entry superseding it. A decision taken while working is added there in the same
pull request as the work, in the form the file already uses — what was decided,
why, and what follows — addressed to nobody and pointing at nothing outside the
clone.

**Planning is not.** `.notes/` is gitignored and excluded from the Nix source
filter and from release archives. It holds `STATUS.md`, `ROADMAP.md`, the plans
under `plans/`, the dated handoffs under `handoffs/`, coordination under
`messages/`, and drafts of decisions not yet taken.

If `.notes/` is present, read `STATUS.md`, `ROADMAP.md` and the active plan
before changing anything, update `STATUS.md` when the active state changes, mark
acceptance criteria in the active plan, and add a dated file under `handoffs/`
after substantial work rather than rewriting an old one.

If `.notes/` is absent — a fresh clone, CI, a worktree — work from the tracked
documentation and the instruction you were given. Do not recreate private
project history from guesses, and do not implement a design you cannot find
authority for in `docs/decisions.md`, the rest of the tracked docs, or what you
were asked for. Ask instead.

Never `git add -f .notes`, never copy note contents into tracked files, and
never store credentials, flags, tokens, or private registry URLs there.

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

## 4. You are not alone in this tree

Assume that a person, or another agent, is editing this repository at the same
time as you. Nothing here is theoretical: this working tree is shared, and a
change that was correct in isolation still destroys somebody's afternoon when it
lands on top of their unsaved work.

Before you start, run `git status`. Every path it lists that you did not touch
belongs to someone else. Do not edit it, do not stage it, do not reformat it,
and do not "fix" it in passing. If your task genuinely needs one of those files,
say so and ask before touching it.

While you work:

- The index is shared as surely as the tree is. `git add` followed by
  `git commit` commits everything else that happens to be staged, including
  work somebody staged a second earlier, so commit with an explicit pathspec:

  ```sh
  git commit -F <message-file> -- AGENTS.md scripts/commit-check.sh
  ```

  That form takes the paths you name and ignores the rest of the index. Never
  `git add -A`, `git add .`, or `git commit -a`.
- A pathspec is not a shield, only a filter: `git commit -- <paths>` takes the
  whole content of the files it names, edits from somebody else included. Read
  `git diff -- <paths>` first and name only files whose diff is entirely yours.
  A file two people are changing at once needs a word between them, not a
  cleverer invocation of git.
- Read `git diff --cached --name-only` as a command of its own, before you
  commit. A check whose output arrives in the same breath as the commit is not
  a check.
- `git commit --amend` rewrites whatever HEAD points at, which is not
  necessarily the commit you just made. Confirm with `git log -1` immediately
  beforehand; a worker who committed in the intervening minute is otherwise
  about to lose their message.
- Re-read a file immediately before editing it if any time has passed since you
  last read it. Prefer a small targeted edit over rewriting a file you only
  partly own.
- Never run `git restore`, `git checkout --`, `git stash`, `git reset`, or
  `git clean` over a path you did not create in this session. There is no undo
  for somebody else's uncommitted work.
- Never amend, reword, or drop a commit you did not author in this session.
- `cargo fmt --all` rewrites every file in the workspace, including the one
  somebody is halfway through. Run `cargo fmt --all -- --check`, as
  `scripts/verify.sh` does, and format only the files you changed.
- Two `scripts/verify.sh` runs at once share `target/` and the fixture trees.
  Cargo's own lock makes concurrent builds wait rather than corrupt, but
  `SLOPIUM_UPDATE_FIXTURES=1 scripts/publish-check.sh` writes into
  `tests/registry` and `tests/consumer` and must never run while another
  regeneration or another agent's checks are in flight.
- If `main` moved while you worked, merge it into your branch. Do not rebase a
  branch you have already pushed, and do not transplant somebody else's commits
  onto it.

When several agents work in parallel, give each one its own `git worktree` and
its own branch, and assign file ownership up front. Two writers in one worktree
is not a workflow. Coordinate through `.notes/messages/` when notes are present;
otherwise say plainly in your report which files you claimed and which you left
alone, so the next worker can read it.

## 5. How a change lands

Every change reaches `main` through a pull request. Nothing is pushed to `main`
directly, nothing is force-pushed, and no published history is rewritten.

**Start from an issue** for anything larger than a typo — a bug report or a
proposal, on the tracker, before the code exists. The issue is where the shape
of a change is argued and the pull request is where the change is read; keeping
them apart is what stops a review from re-deciding the design. A pull request
that closes one says `Fixes #N` in its body.

**Cut a branch from current `main`**, named `type/short-kebab`, `type` being one
of the commit types in §6. Lowercase ASCII, digits and hyphens, at most 48
characters. For anything that touches three or more of {compiler, manager,
manifest layer, runtime, standard library, editor plugin}, or that combines a
format change with a behavior change, present a plan first: goals, the order of
the work, and what "done" means.

WIP and fixup commits are fine while the branch is yours alone, and must not
survive it. A pull request lands **one commit, or a few that each stand on
their own**; anything else is squashed before it is marked ready.

**A pull request merges as a merge commit** — never a squash, never a rebase.
The merge commit's subject is the pull request's title, which is why the title
obeys §6 like any other subject, and its body is empty, because the prose
belongs to the commit underneath it and a review checklist is not history. The
result reads with `git log --first-parent`: one line per pull request, and the
detail one level down.

Open it as a draft while it settles and mark it ready when §9 is green locally.
The description says what changed, why, how it was verified, and what could not
be verified here. It is written for a reviewer and it obeys §6's rule about
addressing no one, because a description outlives the review that prompted it.

Merging needs green CI and §11. The branch is deleted afterwards. If `main`
moved while you worked, merge it into your branch rather than rebasing commits
that are already pushed.

## 6. Commit contract

This is the part a contributor cannot guess from the code, so it is spelled out
completely. Read `git log --first-parent` for calibration.

### Subject

```text
type(scope): imperative lowercase description
```

- Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `build`, `ci`,
  `chore`, `revert`.
- Scopes in use: `slopium` (the language and the toolchain — the default for
  compiler, manager, standard library, and runtime work), `slopic` for work that
  is the compiler's alone, `manifest`, `docs`, and `release` for the one commit
  §7 describes. Ask before inventing a new one, and add it to
  `scripts/commit-check.sh` in the same commit.
- English, imperative, lowercase after the colon, no trailing period.
- Aim for 72 characters, hard limit 95.
- Say what the language or the toolchain now does, not which files moved. The
  house form is two clauses naming the two things that changed:

  ```text
  feat(slopium): read what a borrow points at, and match without taking it apart
  feat(slopium): print a float exactly, without a C library under it
  fix(slopium): resolve a shared dependency under every namespace
  ```

- Never `feat(slopium): update sema.rs`, never `various fixes`, and never a
  version number — `chore(release): vX.Y.Z` in §7 is the single exception.
- A pull request's title is a subject in this form, because the merge commit
  takes it verbatim, together with the ` (#N)` the forge appends.

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
- **A message addresses nobody.** No "you", no "as discussed", no "as
  requested", no "I decided", no crediting or thanking a person, no answering a
  question. It is written about the software, for a stranger reading `git log`
  in five years who was in no conversation and cannot tell whose idea anything
  was. Where the reason for a change came out of a discussion, state the reason
  and never its source.
- **Point only at what a clone contains.** A path into `.notes/`, a planning
  file, a chat log: the reader can open none of them, so the substance is
  written out instead. A decision identifier such as `D-106` may be cited,
  because `docs/decisions.md` is in the clone, and the sentence around it still
  has to make sense without following it.
- The only trailers are `Fixes #N` and `Refs #N`, for an issue that exists, and
  `BREAKING CHANGE:` alongside a `!` in the subject, agreed before the work
  starts. No `Signed-off-by`, no `Co-authored-by`, and no AI or agent
  attribution — not once, not even when a default tells you to.

A merge commit is the exception to all of the above: its subject is the pull
request's title and its body is empty, so there is nothing there to write.

If the explanation does not fit in two paragraphs, it is not a longer commit
message. It is a `docs/` change when a user of the language needs it, and a
`.notes/` entry when it is project reasoning.

### Checked, not merely stated

`scripts/commit-check.sh` decides the half of this contract a machine can
decide — the subject's shape, mood and width, the blank second line, a prose
body wrapped at 80 in at most two paragraphs, the trailers, the forms of
address, and the version rule of §7. What it cannot decide is whether the
paragraphs are worth reading; that stays yours.

```sh
scripts/install-hooks.sh                 # once per clone: the commit-msg hook
scripts/commit-check.sh                  # origin/main..HEAD
scripts/commit-check.sh --message .git/COMMIT_EDITMSG
```

CI runs it over the commits a pull request adds. History before v0.4 predates
the contract and does not satisfy it, which is why the check is given a range
rather than let loose on the whole log.

## 7. Versioning and releases

The version in `Cargo.toml` is a property of the last release rather than of the
last commit, and the manifest is the only place it is written down: `flake.nix`
reads it from there, and so does everything that repeats it.

**No ordinary commit touches `workspace.package.version`.** A `feat` used to be
a release here, bumping the version in its own commit and taking a tag. That
cannot survive more than one branch at a time: two pull requests would both
claim the next number, whichever merged second would be claiming a version that
already existed, and both would have regenerated
`tests/consumer/Slopium.lock`, which carries the version and the bundled
library's checksums, into a conflict. What a change owes instead is one file
under `changelog.d/`, whenever somebody using the language or the toolchain
would notice it.

**A changelog entry is a file, for the same reason the version is not a
commit** (`D-137`). Every entry written under `[Unreleased]` went at the same
line, so every two changes in flight conflicted there over nothing — the
entries never disagreed, they were merely inserted at one anchor. A change
writes `changelog.d/<issue>[-<discriminator>].<kind>.md` instead, holding one
bullet exactly as it will be published: a new file collides with no other.
`[Unreleased]` keeps its heading and holds nothing, and
`scripts/changelog-check.sh` says so. `changelog.d/README.md` is the whole rule
for somebody writing one.

Pre-1.0 semantics, decided at the release out of what the changelog collected:
minor for a language or toolchain capability, patch for a behaviour fix inside
an existing capability.

A release is its own pull request, and it moves more than the manifest:

1. cut `release/vX.Y.Z` from current `main`;
2. set `workspace.package.version` and let `Cargo.lock` follow;
3. regenerate the committed registry and consumer with
   `SLOPIUM_UPDATE_FIXTURES=1 scripts/publish-check.sh`, because the bundled
   library's digest moves with the version;
4. collate the entries with `scripts/changelog-collate.sh vX.Y.Z`, which
   writes the `[X.Y.Z] - <date>` section under `[Unreleased]`, adds the link
   reference, and empties `changelog.d/`;
5. run `scripts/release-check.sh --check-release vX.Y.Z`, then §9 in full;
6. commit as `chore(release): vX.Y.Z` and merge the pull request;
7. tag the merge commit `vX.Y.Z`, annotated, and push the tag.

Pushing the tag is what publishes. `.github/workflows/release.yml` re-checks
that the tag, the manifest and the changelog agree, runs the suite, builds the
toolchain, and leaves a **draft** release with the artifacts and their checksums
attached. A person presses publish, because a release is the one thing in this
repository that another commit cannot undo.

**The release page is generated, and that is deliberate.** Its first half is
one line per pull request the tag adds, taken from the merge commits with
`git log --first-parent`, so a pull request's title is published verbatim
wherever anybody looks at a release. Nobody rewrites it there afterwards; the
place to make it read well is the title, before the merge (`D-131`). Its second
half is that version's `CHANGELOG.md` section, which is the part a person
writes and says why a change was worth making.

Proposing a release is yours; performing one is not. Do not set a version, cut a
release branch, or create a tag without being asked to. Tags up to `v0.9.2` are
lightweight; every tag from now on is annotated.

## 8. What must change together

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
| A design question you had to decide to finish the work | a new entry in `docs/decisions.md` |
| A CLI flag or subcommand | the clap definition, generated completions, and `README.md` |
| Anything a user of the language sees | `README.md` (Russian) |
| Anything a user of the language or the toolchain would notice | a file under `changelog.d/`, named for the issue |
| The commit contract, the branch and pull-request flow, or the release steps | `scripts/commit-check.sh`, `scripts/release-check.sh`, `CONTRIBUTING.md`, the templates under `.github/` |

Regenerate the committed registry and consumer with:

```sh
SLOPIUM_UPDATE_FIXTURES=1 scripts/publish-check.sh
```

Their being byte-identical on a re-run is itself an assertion — reproducible
archives and deterministic Ed25519 signing. A diff there is a regression, not a
timestamp.

## 9. Validation

The gate for a code change is the full suite, from the repository root:

```sh
scripts/verify.sh
```

It runs `release-check.sh --check`, `changelog-check.sh`,
`cargo fmt --all -- --check`,
`cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`, then
`project-tests.sh`, `package-check.sh`, `git-check.sh`, `registry-check.sh`,
`publish-check.sh`, `runtime-check.sh`, `core-check.sh`, `debug-check.sh`,
`cross-check.sh`, `object-check.sh`, and finally `nix flake check`.

Rules:

- A failing required check blocks the merge. If the failure is demonstrably
  pre-existing, show before-and-after evidence and ask.
- Without valgrind, gdb, qemu, or the AArch64 toolchain, thirteen places in the
  suite print a line and pass. `SLOPIUM_STRICT=1 scripts/verify.sh` turns every
  one of them into a failure. Use it when you want to know that the suite ran
  rather than that it finished.
- If you could not run part of the suite, say exactly which part and why in your
  report. A skipped check is never reported as a passing one.
- For a docs-only change, a content and Markdown review plus `git diff --check`
  is enough.
- CI runs the whole of `scripts/verify.sh` inside `nix develop` with
  `SLOPIUM_STRICT=1`, so nothing is skipped there, plus faster fmt/test/clippy
  and `project-tests.sh` jobs for early signal, the commit contract over the
  commits the pull request adds, and `scripts/release-check.sh --check`.
  Passing locally is still the standard; CI is the backstop.
- The dev shell is where the suite is whole: `nix develop` carries valgrind,
  gdb, qemu, and a cross toolchain named `aarch64-unknown-linux-gnu-*`, which
  is the prefix `core-check.sh` and `object-check.sh` look for.

## 10. Code rules

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

## 11. Definition of done

- [ ] `scripts/verify.sh` is green, or the exact subset that ran, and why, is in
      the pull request.
- [ ] The companions in §8 are updated.
- [ ] `changelog.d/` has this change's entry, unless nobody using the language
      or the toolchain would notice it — and `CHANGELOG.md` itself is untouched,
      because only a release writes there.
- [ ] `git status` and `git diff --cached --name-only` reviewed in their own
      right — every path is one you touched, and none is `.notes/`, build
      output, or a stray fixture.
- [ ] The commit names its paths: `git commit -F <message-file> -- <paths>`.
- [ ] The version is untouched — only the release pull request of §7 moves it.
- [ ] Subject: conventional, imperative, lowercase, at most 95 characters, about
      behavior rather than files.
- [ ] Body: one or two paragraphs, wrapped at 80, no lists, no trailer beyond
      `Fixes #N`, addressing nobody, pointing at nothing outside the clone —
      `scripts/commit-check.sh` agrees.
- [ ] Nothing pushed, merged or tagged that was not asked for.

# Contributing to Slopium

Thanks for looking. This page is the short path from a clone to a reviewable
pull request. The rules a change is held to live in [AGENTS.md](AGENTS.md);
this is how to walk through them.

## Get it building

Everything is reproducible through the Nix dev shell, and the check suite is
only whole inside it: it carries valgrind, gdb, qemu and an
`aarch64-unknown-linux-gnu-*` cross toolchain, and thirteen checks quietly skip
themselves without those.

```sh
git clone https://github.com/keepinfov/slopium
cd slopium
nix develop
scripts/install-hooks.sh     # once per clone: the commit-msg hook
```

Then:

```sh
cargo test --workspace                       # the Rust side
scripts/project-tests.sh                     # the language fixtures
SLOPIUM_STRICT=1 scripts/verify.sh           # everything, and no silent skips
```

A plain `cargo build` works without Nix, and so does most of the suite; say
which parts you could not run rather than leaving the question open.

## Find something to do

- Bugs and proposals live in the
  [issue tracker](https://github.com/keepinfov/slopium/issues). Open one before
  writing a large feature, so the design is settled before the code exists.
- Both kinds go through a form. The questions are not ceremony: the toolchain
  version decides which language you are describing, the target decides which
  backend emitted the code, and a minimal `.slp` is what turns a report into a
  fixture.
- New to the codebase? `docs/architecture.md` is the compiler end to end,
  `docs/language.md` is the language, and `tests/README.md` says how the
  fixtures work. `docs/decisions.md` is why any of it is the way it is — read
  the entry before reopening a question it already answers.

## Make the change

```sh
git switch -c fix/short-description main
```

Three things a reviewer looks for immediately:

1. **A test.** A fix comes with a fixture that fails without it; a feature comes
   with fixtures for what it now does. A refusal comes with a stable `SL` code
   and a `compile_fail` case.
2. **The companions.** AGENTS.md §8 is a table of what has to move with what —
   syntax touches both backends, the docs, the editor word lists and a fixture,
   and forgetting one is the most common way a change is incomplete.
3. **A changelog line** under `[Unreleased]`, if a user of the language or the
   toolchain would notice.

Commit subjects describe the state after the change rather than the act of
changing it — `feat(slopium): read what a borrow points at, and match without
taking it apart`, not `add borrow reads`. The full contract, including the body,
is AGENTS.md §6, and `scripts/commit-check.sh` decides the half of it a machine
can.

## Open the pull request

Draft while it settles, ready when the suite is green. The title is the subject
of the merge commit, so it obeys the commit contract; the description says what
changed, why, how it was verified, and what could not be verified here.

**Your title is published.** The release page for whichever version your change
lands in is generated from the merge commits it added, one line each, and
nobody edits it afterwards — a vague title stays vague in front of everyone who
downloads the toolchain. Write it for that reader (`D-131`).

CI runs formatting, clippy, the Rust tests, the language fixtures, the commit
contract, and the whole of `scripts/verify.sh` inside the dev shell with
`SLOPIUM_STRICT=1`.

A pull request merges as a merge commit — not a squash — so keep the branch to
one commit, or a few that each stand on their own.

## House rules worth repeating

- Do not weaken, delete or rewrite a test, a fixture or an expectation file to
  make a check pass. If an expectation is wrong, say why before changing it.
- Do not reformat or refactor code your change does not touch. `cargo fmt --all`
  rewrites the whole workspace; format only your files.
- No secrets anywhere — not in source, not in a log excerpt, not in an issue.
  The Ed25519 key in `scripts/publish-check.sh` is a deliberate public fixture
  and signs nothing real.
- **Point only at what a clone contains.** A private notes directory, a planning
  file, a chat log: whoever reads your commit or pull request cannot open any of
  them, so write the substance out.
- The version in `Cargo.toml` belongs to the release, not to your change. Never
  bump it in an ordinary pull request; AGENTS.md §7 is how a release is cut.

## Licence

Slopium is [MIT](LICENSE). By contributing you agree your work is released under
it.

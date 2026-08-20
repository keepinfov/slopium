<!--
The title is the subject of the merge commit, so it obeys AGENTS.md §6:
`type(scope): imperative lowercase description`, at most 95 characters, about
behaviour rather than files.

Everything below is for the review. It does not reach the history.
-->

## What is now true that was not

<!-- One or two paragraphs. The same ones the commit body carries. -->

## How it was verified

<!-- The commands that ran, and what they said. -->

- [ ] `scripts/verify.sh` green, or the exact subset named below.

Not run here, and why:

<!-- e.g. "no aarch64 toolchain, so cross-check.sh skipped its second half" -->

## Definition of done (AGENTS.md §11)

- [ ] Companions updated — the table in [AGENTS.md §8](https://github.com/keepinfov/slopium/blob/main/AGENTS.md#8-what-must-change-together).
- [ ] A test that fails without the change, or a fixture for the new behaviour.
- [ ] A stable `SL` code and a `compile_fail` case, if the compiler refuses
      something new.
- [ ] An entry under `changelog.d/`, if a user would notice. Not `CHANGELOG.md`,
      which only a release writes.
- [ ] The version in `Cargo.toml` is untouched — only a release moves it.
- [ ] No secret, no build output, no private path anywhere in the diff.

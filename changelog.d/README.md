# Changelog fragments

A change writes its changelog entry here, in its own file, rather than under
`[Unreleased]` in `CHANGELOG.md` (`D-137`). Two changes in flight then collide
over nothing: a new file conflicts with no other.

Name the file after the issue the change closes:

```text
changelog.d/6.added.md
changelog.d/44-fold.fixed.md
```

`<issue>[-<discriminator>].<kind>.md`. The kind is one of `added`, `changed`,
`deprecated`, `removed`, `fixed` and `security`. The discriminator is for a
change that owes two entries of the same kind, and is otherwise left out.

A fragment holds **one bullet, written exactly as it will be published**:
starting `- `, continuation lines indented two spaces, wrapped at 80 columns.
Nothing reflows it, so the width you write is the width that ships. Say what a
user of the language or the toolchain can now do that they could not, in the
voice `CHANGELOG.md` already uses.

A change nobody using the language or the toolchain would notice owes no
fragment.

`scripts/changelog-check.sh` decides the shape, and runs from
`scripts/verify.sh`. A release turns every fragment here into that version's
section with `scripts/changelog-collate.sh vX.Y.Z`, which is step 4 of
`AGENTS.md` §7 and empties this directory.

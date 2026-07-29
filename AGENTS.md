# Agent workflow

Project planning is local and intentionally excluded from Git.

Before changing the repository:

1. If `.notes/` exists, read `.notes/STATUS.md`, `.notes/DECISIONS.md`, and
   `.notes/ROADMAP.md`.
2. Read the active file under `.notes/plans/` relevant to the task.
3. Do not implement syntax decisions marked as proposed or deferred.

While collaborating:

- write directed coordination notes under `.notes/messages/`;
- update `.notes/STATUS.md` when the active state changes;
- finish substantial work with a dated file under `.notes/handoffs/`;
- record accepted or rejected design choices in `.notes/DECISIONS.md`;
- never run `git add -f .notes` or copy local notes into public documentation;
- never store credentials, flags, tokens, or other secrets in `.notes`.

If `.notes/` is absent (for example, in a fresh public clone), continue using
the tracked documentation and the user's instructions without recreating
private project history from guesses.

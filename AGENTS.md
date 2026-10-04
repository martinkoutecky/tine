# Tine — agent working agreement (pointer)

The full, canonical agent working agreement for this project lives **outside this
git repository** — deliberately, so that a `git clean` in the worktree can't
delete it (that happened on 2026-07-10 and wiped the in-tree copy). Read it before
working (paths below are relative to the repository root):

```
../tine-agents/AGENTS.md
```

Private engineering & data-safety specs live alongside it under
`../tine-agents/specs/` (audits, perf-batch, data-safety fixes,
notes). The public roadmap is `docs/BACKLOG.md`; architecture decisions are in
`docs/adr/`.

This pointer is intentionally **tracked** (a tracked file survives `git clean`, an
ignored one does not), so both agents — Codex (reads `AGENTS.md`) and Claude Code
(reads `CLAUDE.md`, which imports the same file) — always discover the working
agreement. The canonical file is local and not part of the public repo. The default is a
sibling `tine-agents` directory; if you keep it elsewhere, set `TINE_AGENTS` to
that directory and read `$TINE_AGENTS/AGENTS.md` and `$TINE_AGENTS/specs/`.
If neither location exists, ask the maintainer for the working agreement.

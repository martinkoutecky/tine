# Tine — agent working agreement (pointer)

The full, canonical agent working agreement for this project lives **outside this
git repository** — deliberately, so that a `git clean` in the worktree can't
delete it (that happened on 2026-07-10 and wiped the in-tree copy). Read it before
working:

```
/aux/koutecky/logseq/tine-agents/AGENTS.md
```

Priority comes from the maintainer's queue (see the agreement), never from a
document. Current model routing is in
`/aux/koutecky/logseq/tine-agents/ROUTING.md`.

Private engineering & data-safety specs live alongside it under
`/aux/koutecky/logseq/tine-agents/specs/` (audits, perf-batch, data-safety fixes,
notes). The public roadmap is `docs/BACKLOG.md`; architecture decisions are in
`docs/adr/`.

This pointer is intentionally **tracked** (a tracked file survives `git clean`, an
ignored one does not), so both agents — Codex (reads `AGENTS.md`) and Claude Code
(reads `CLAUDE.md`, which imports the same file) — always discover the working
agreement. The canonical file itself is local to this machine and not part of the
public repo.

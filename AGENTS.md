# Tine — agent instructions (pointer)

## Outside contributors: you can start now

The maintainer's private working agreement (below) is **not needed** to read,
explore or change this code. If `../tine-agents/AGENTS.md` does not exist, do
not stop and do not recreate it. Work from the public material instead:

- `CONTRIBUTING.md`: how changes are proposed and reviewed here.
- `README.md`: what Tine is and how to build it.
- `docs/BACKLOG.md`: product direction; `docs/adr/`: architecture decisions;
  `docs/storage-sync-contract.md` and `docs/contracts/`: living contracts.
- Build: `npm ci`, then `npm run build`; the desktop app is the Tauri crate in
  `src-tauri/`. Tests: `npm test` and `npx tsc --noEmit` for the frontend,
  `cargo test` for the Rust crates. Format Rust only with `cargo fmt --all`.
- Tine writes the user's real Logseq graph. Never add a file-write path that
  bypasses the existing guarded save path (temp file, fsync, rename,
  base-revision check).

## The maintainer's working agreement (optional)

The maintainer's full agreement lives **outside this git repository**, so that
a `git clean` in the worktree cannot delete it (that happened on 2026-07-10).
When present, read it before working (paths relative to the repository root):

```
../tine-agents/AGENTS.md
```

Private engineering and data-safety specs live alongside it under
`../tine-agents/specs/`. If you keep it elsewhere, set `TINE_AGENTS` to that
directory and read `$TINE_AGENTS/AGENTS.md` and `$TINE_AGENTS/specs/`.

This pointer is intentionally **tracked** (a tracked file survives `git clean`;
an ignored one does not), so both Codex (reads `AGENTS.md`) and Claude Code
(reads `CLAUDE.md`, which imports the same file) discover the agreement when it
exists.

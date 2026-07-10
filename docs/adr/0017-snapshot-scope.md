# 0017. Launch snapshots cover graph text + config + PDF highlight sidecars — not asset bytes

Date: 2026-07-02

## Status

Accepted

## Context

ADR 0007 promises launch snapshots with one-click restore. The implementation
snapshotted only `journals/` + `pages/` (`.md`/`.org`) and `logseq/config.edn`.
But `assets/` also holds *mutable user data*: PDF highlight sidecars
(`assets/*.edn`, rewritten by 3-way merges on every highlight edit) and
area-highlight crops. The 2026-07 audit flagged the mismatch between the
invariant's wording ("a safety snapshot you can restore") and its actual scope.

## Decision

- Snapshots (and restore, and the pre-restore snapshot) include: page/journal
  text, `logseq/config.edn`, and **`assets/**/*.edn` highlight sidecars** —
  everything Tine itself rewrites in place.
- **Binary asset bytes stay excluded, by decision:** they are write-once in
  practice (paste/import creates them; nothing rewrites them in place — the
  asset paths use `create_new` reservation + atomic copy), and copying
  media/PDF binaries on every launch would make snapshots unaffordably heavy.
- Anything user-facing that mentions snapshots/restore must state this scope
  ("notes, config, and PDF highlights — not media files") rather than implying
  whole-graph coverage.
- A restorable snapshot is published only after every selected file was copied and
  a versioned manifest binding it to the canonical graph root, with a SHA-256
  inventory of every file, was fsynced. Partial, modified, or legacy-unverified
  directories are not offered as normal restore points.
- The manifest records the snapshot's configured page/journal directories; restore
  uses those recorded graph-relative paths and rebuilds the live `Graph` afterward.

## Consequences

- A corrupted highlight write is now recoverable from the last launch snapshot.
- The known residual: a deliberately overwritten/trashed *binary* asset is not
  snapshot-recoverable (trash covers the delete path; nothing rewrites asset
  bytes in place). If Tine ever gains an asset-rewriting feature, this ADR must
  be revisited first.

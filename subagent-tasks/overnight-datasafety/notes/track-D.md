# Track D — DS#5: snapshot journals before launch filename migration

Graph load now checks whether any journal filename would be migrated before doing filesystem renames; only when a candidate exists does it synchronously reuse the normal launch-backup path to snapshot the graph into the restore-visible backup area first, then runs the existing `migrate_journal_filenames()` rules unchanged and skips the later async launch backup to avoid double-backup. The common no-migration launch path still only does the O(n) candidate scan and keeps the existing async launch backup. Restore confirmation copy in `src/components/Settings.tsx` remains true: restores still snapshot current state first and are reversible through the normal backup UI. Necessity test was written first: `cargo test -p tine graph_load_snapshots_original_journal_filename_before_migration` failed red pre-fix because the backup lacked `Thursday, 25-06-2026.org`; post-fix it passes and the migrated live file `2026_06_25.org` still exists. SHA: final commit SHA printed in the Track D handoff (`DONE_TRACK_D`).

## Touched Files

- `crates/tine-core/src/model.rs`
- `src-tauri/src/backup.rs`
- `src-tauri/src/graph.rs`
- `subagent-tasks/overnight-datasafety/notes/track-D.md`

## Verification

- RED before fix: `cargo test -p tine graph_load_snapshots_original_journal_filename_before_migration` -> failed, `backup must contain the original pre-migration filename`.
- GREEN after fix: `cargo test -p tine graph_load_snapshots_original_journal_filename_before_migration` -> 1 passed.
- `cargo test -p tine` -> 21 passed.
- `cargo test -p tine-core migrate_recovers_title_named_org_journals` -> 1 passed, 291 filtered out.
- `cargo test -p tine-core` -> 291 passed, 1 ignored.

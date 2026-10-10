# 0062. Launch snapshots cover graph text across the whole graph

- **Status:** Accepted — Martin, 2026-09-29 (og QUESTIONS Q8 for the
  outcome; this ADR, including the schema change and the added persisted
  format, accepted the same day). He also confirmed that og never prunes
  master's snapshots: rare in practice, and the safer rule.
- **Amended 2026-10-10 (og-backup-cas, design approved by Martin):** og now
  writes schema 4, which keeps this ADR's scope, policy and `graph/` paths but
  stores each distinct content once in `blobs/<sha256>` beside the snapshots;
  the snapshot directory holds only the manifest. og still lists, restores
  and prunes schema 3 and 2. Contract: docs/storage-contract.md, Graph
  backups.
- **Date:** 2026-09-29
- **Unit cost:** 0 bytes and 0 files per edit on a 1-block or a 60-block page
  (snapshots are taken at launch and before user-requested rewrites, never per
  edit); no transport. Per snapshot on a copy of the anonymized graph (1,075
  text files): text copy unchanged at 1,075 files / 1,300,622 bytes, manifest
  +6,574 bytes (154,891 vs 148,317). Measured by snapshotting the copy with the
  old and new writers; details under Unit cost below.

## Context

Tine snapshots the graph into `<app data>/backups/<graph-id>/<stamp>/` at each
launch and before a rewrite the user asked for. Restore puts a snapshot back.
Until this change og's snapshot (schema 2) copied only the configured
`journals/` and `pages/` directories. Logseq and Tine both read Markdown/Org
anywhere in the graph-text scope, for example a page at the graph root or under
`archive/`. Those pages had no snapshot, and a restore could not bring them back.

Master fixed this in `ffb4cb3d716c` ("Snapshot graph text across the full
graph") and then `336833b13ce1`. Master's schema 3 keeps text at
`graph/<graph-relative path>` and records the graph-text scope it covered. Its
restore puts each file back at that path. It retires live text that the
snapshot does not list, but only inside the recorded scope. It ignores the
recorded pages and journals directories. Master still reads schema 2. og shares
the app-data directory with a released master build (docs/app-identity.md).
Before this change og treated schema 3 as foreign: it did not list, restore or
prune those snapshots.

Alternatives on the table:

1. **Copy the extra folders into the schema-2 layout.** Schema 2 has no place
   for a root page, and master would misread the snapshot. Rejected.
2. **Reuse master's schema 3 wire format.** Each build then reads the other's
   snapshots. Chosen.

## Decision

**Snapshot (schema 3).**
- Every Markdown/Org file that og's discovery admits is copied to
  `graph/<graph-relative path>`. Discovery here means `graph_text_eligible`,
  exposed as `tine_store::Area::Graph`.
- Asset `.edn` sidecars and `logseq/config.edn` are copied as before.
- The manifest adds `graph_text_policy: {version: 2, hidden, hidden_parse_failed_closed}`,
  in master's field names and with master's `GRAPH_TEXT_SCOPE_VERSION`.
- og records `hidden_parse_failed_closed` as master does. (Amended og T2,
  2026-09-29: og's `:hidden` failed open on an invalid value and always
  recorded `false`; it now fails closed like master.)
- og adds `"writer": "og"`. Master's serde ignores unknown fields.
- A snapshot still refuses to publish when the copied count differs from the
  live count.

**Restore.**
- Schema 3 goes through `Store::restore(kind, files, Some(recorded hidden))`.
  Text returns to its exact graph-relative path.
- Unlisted live text inside the recorded scope moves to
  `logseq/.tine-trash/<restore-id>/graph/<path>` and is never deleted.
- A master snapshot with `hidden_parse_failed_closed: true` holds no text. It is
  restored with the scope "hide all" and retires nothing.
- The recorded `pages_dir`/`journals_dir` do not steer a schema-3 restore.
- Schema 2 keeps its configured-roots restore and still refuses a snapshot made
  under a different directory setting.
- Both schemas go through the one guarded restore path: no-replace publish,
  fsync, and recovery before retirement.
- A pre-restore safety snapshot of the whole graph is required, as before.

**Refusals (in-scope scenarios, `docs/storage-contract.md`).**
- **Malformed or mixed snapshot content.** The restore is refused before any
  live file moves if:
  - a whole-graph restore is given Pages/Journals input, or Graph input
    outside the recorded scope;
  - a configured-roots restore is given Graph input.
- **A snapshot area missing from disk (disk error, partial copy).** If
  `graph/` (schema 3) or `journals/`/`pages/` (schema 2) is missing, the
  restore is refused. Otherwise the missing area would read as "no files" and
  retire every live one.

**Prune.**
- og's keep-count counts schema-2 snapshots and schema-3 snapshots marked
  `writer: "og"`.
- Master's schema-3 snapshots are listed and restorable in og. og never prunes
  them.

## Consequences

**Interoperability.**
- Root and other-folder pages now survive a bad write or accidental edit.
- og restores master's schema-3 snapshots.
- Master restores og's schema-3 snapshots. (Amended 2026-10-10: og's
  schema-4 snapshots are og-only; docs/app-identity.md.)

**Scope reading matches master's.**
- og reads the recorded `hidden` list with `configured_hidden`, the one
  `:hidden` reader that discovery, the watcher, snapshot capture and restore
  share.
- Master's `GraphTextScope` trie is also a byte-exact prefix match: neither
  folds case, both ignore one trailing `/`, both treat malformed aliases as
  inert, and an empty entry hides everything.
  `store_read.rs::hidden_prefix_answers_match_master_for_listing_and_discovery`
  pins these answers against master's own test cases.
- An entry with leading or trailing Unicode whitespace (for example a
  no-break space) is inert in both (og T2 aligned og with master's
  `lexical_components`; `store_read.rs::hidden_entry_with_unicode_edge_whitespace_is_inert`).
- A malformed or over-limit `:hidden` value hides all graph text in both, and
  a snapshot taken then records `hidden_parse_failed_closed` (og T2).
- A snapshot file that og's reading puts outside the scope makes the restore
  refuse, not partly apply.

**Config directory change.**
- `Store::restore` still refuses a supplied `config.edn` that changes the
  pages/journals directories.
- Master's schema-3 restore allows that change. Recorded as a known
  difference.

**Cost.** Restore walks the graph root (minus `assets/`, dot directories and
Tine's internal trees) to find stale text. This replaces the walk of two
directories. The cost is O(graph directory entries) and the walk reads no
file content.

**Prune.** Master's keep-count counts og's snapshots, as it already did for
og's schema-2 snapshots. og never counts master's.

**Tests.** The persisted-format census gains `backup-graph-text-copy`
(26 → 27).

## Unit cost

Measured 2026-09-29 on a copy of the anonymized graph: 1,075 Markdown files,
1,300,622 bytes, all under `pages/` and `journals/`. The measurement ran og's
`write_snapshot` and the base's two `copy_store_area` calls on the same store.

- **Per edit:** 0 bytes, 0 files, 0 transport bytes, on a 1-block and on a
  60-block page. Snapshots are taken per launch and before a user-requested
  rewrite, never per edit.
- **Per snapshot, this graph:**
  - 1,075 text files and 1,300,622 text bytes, identical to schema 2.
  - The manifest is 154,891 bytes. A schema-2 manifest of the same snapshot
    would be 148,317 bytes.
  - The difference is +6,574 bytes: a 6-byte `graph/` prefix on 1,075 paths,
    plus the policy and writer fields.
  - Total: 1,076 files, 1,455,513 bytes.
- **Per snapshot, a graph with text outside the roots:** the snapshot grows by
  exactly those files and their bytes. Before this change those files had no
  snapshot.
- **Transport:** none. Snapshots stay in app data, outside the graph.

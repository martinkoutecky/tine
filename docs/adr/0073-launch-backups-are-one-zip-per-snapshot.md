# 0073. Launch backups are one verified zip per snapshot

- **Status:** Proposed. Outcome decided by Martin on 2026-10-11 ("for now do full zips"). The schema-5
  persisted format in this ADR needs his acceptance (§12).
- **Date:** 2026-10-11
- **Unit cost:** 0 bytes, 0 files and 0 transport bytes per edit on a 1-block or a 60-block page. Snapshots
  are taken at launch and before a rewrite the user asked for, never per edit. Per launch on a copy of the
  anonymized graph (1,075 text files, 1,300,622 bytes):
  - first snapshot: 1 zip of 918,588 bytes, 4 syncs;
  - unchanged launch: 0 files, 0 bytes, 0 syncs;
  - after 3 page edits: 1 zip, 2 syncs.

  The schema-3 directory it replaces wrote 1,076 files, 1,455,488 bytes and 1,083 syncs on every launch.
  Measured 2026-10-11 with `launch_backup_cost_on_corpus`; the table is in `docs/storage-contract.md`
  under "Graph backups".

## Context

Every graph open snapshots the graph into app data (ADR 0062, schema 3). Each snapshot is a directory
holding a copy of every graph text file and asset sidecar, plus `logseq/config.edn`, with a `sync_all` per
file and per directory. On the anonymized graph that is 1,076 files and 1,083 syncs per launch, about
0.7–2 s, and slower on Windows, where Defender scans each new file. It happens even when nothing changed
since the last launch.

The first replacement, a content-addressed store (og-backup-cas, schema 4), shared blobs between
snapshots. It failed four reviews on the garbage collection of shared blobs and is parked.

Martin's requirement (2026-10-11): users want raw Markdown they can open without Tine, for when Tine
broke after an update or a graph is too damaged to launch Tine on. Zipping the files is fine. The win to
keep is one file per snapshot.

Alternatives considered:

1. **Keep schema-3 directories and skip unchanged launches.** This removes the unchanged cost, but an
   edited launch still writes about 1,080 files and syncs.
2. **Content-addressed blobs (schema 4).** Writes only changed bytes, but needs a blob GC whose safety
   failed review repeatedly. Parked.
3. **One full zip per snapshot (schema 5).** One file and two syncs per written snapshot, opened by any
   zip tool. An unchanged launch writes nothing. **Chosen.**
4. **Incremental zips (a chain of change sets).** Smaller still, but restore needs the chain. Parked on
   the queue card "Incremental launch backups". The schema-5 manifest (`files: [{path, len, sha256}]`)
   can extend to it.

## Decision

We will write each snapshot as one zip, `<app data>/backups/<graph-id>.zips/<name>.zip`:

- **Layout.** Entries sit at their graph-relative paths. The manifest `tine-backup.json` is the last
  entry: schema 5, the existing scope fields, `files: [{path, len, sha256}]` and a canonical-JSON SHA-256
  `checksum`.
- **Codec.** Tine's own writer and validator over pure-Rust raw Deflate (`flate2` `rust_backend`). It
  has no C codec, writes no ZIP features a built-in extractor might not handle, and never calls a
  generic extraction API. Reads are bounded, and the writer and reader share the same limits, so
  anything Tine writes, Tine can restore.
- **Publication.** A `create_new` partial is synced, then renamed without replacing, then the directory is
  synced. The platform matrix and the Windows recipe are in the contract.
- **Unchanged launches.** A launch whose capture equals the newest snapshot's manifest writes nothing,
  but only when that snapshot is a zip that fully verifies.
- **Lock.** One OS file lock per graph namespace serializes writers, prunes and restore reads across
  Tine processes. Without it, a prune in one process could delete the only verified snapshot while
  another process's newer one is still unsynced.
- **Prune.** Deletion requires a keeper: the newest healthy zip, fully verified and strongly synced.
  Damaged zips are deleted only once older than the oldest retained snapshot. `pre-restore` snapshots,
  another writer's zips and unknown names are never deleted.
- **Restore.** Restore verifies the whole zip, then hands `Store::restore` the verified bytes rather than
  rereading files. It refuses a damaged zip, an unsupported schema, another graph, another assets
  directory setting, and on case-folding platforms names that collide by case (on Apple
  platforms, also by Unicode normalization). Any failure of the
  pre-restore safety snapshot refuses the restore (D5).
- **Legacy.** Schema-2/3 directories stay listed, restorable and pruned. Tine no longer writes them and
  does not migrate them.
- **Settings.** The list shows each backup's status (Damaged, Made by a newer Tine) and restores by a
  namespace-qualified id (`zip:` / `dir:`).

This supersedes ADR 0062 only as the write format. 0062's scope rules (graph-text scope, the recorded
policy, restore retiring only inside the recorded scope) carry over unchanged into schema 5.

## Consequences

- **Cost.** A launch costs one zip and two syncs, or nothing when unchanged. The keep-count now holds the
  last N distinct states, not the last N launches, which only lengthens history (approved 2026-10-11).
- **Readable without Tine.** A snapshot opens in any archive tool. The Guide's "Where your backups are"
  names the folder per platform. There is no in-app "open backups folder" action. On Android and iOS
  the folder is in private app storage.
- **Bounded size.** A graph whose text exceeds the shared limits (128 MiB per file, 512 MiB in total)
  can no longer be snapshotted; the snapshot fails visibly instead of writing an archive restore would
  refuse. Schema 3 had no total limit.
- **Weak durability.** On a filesystem without directory sync, snapshots still publish but prune
  nothing, so they accumulate until a keeper can be certified.
- **Older builds.** Builds that read only schema 2/3 (v0.6.98 stable and earlier Betas) never open
  `<graph-id>.zips/`. They cannot list or restore zips, and they cannot prune them either.
- **Surface.** `tine-store` exposes `DirectoryWitness` and `sync_directory_witness` (Save concept 5 → 7)
  so that backups can distinguish weak durability from strong. `RestoreFile` carries `bytes` instead of
  a source handle and length (Restore 15 → 14).

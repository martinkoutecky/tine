# 0070. A dumb launch checkpoint serves the last published generation at launch

- **Status:** Proposed. Martin decided the design on 2026-10-02 (SPEC-storage
  §7.6). This implementation departs from it in one place: memos are not
  persisted (see Consequences), so Martin accepts or amends it.
- **Date:** 2026-10-02
- **Unit cost:** no per-edit write. Each checkpoint rewrites the whole dump:
  - g13k (13,000 pages): 114.2 MiB raw (119,783,567 B), 19.5 MiB on disk
    (20,452,145 B), written in 780 ms;
  - the anonymized real graph (1,075 pages): 4.4 MiB raw (4,606,396 B),
    1.06 MiB on disk (1,109,631 B), written in 34 ms.

  These are release builds, median of 3, measured by
  `crates/tine-store/examples/checkpoint_launch_bench.rs`; the byte counts are
  identical across runs. Each checkpoint writes 1 file (plus its temporary
  sibling, renamed over it). Transport bytes are 0, because the checkpoint is
  never synced.

  **Frequency bound:** at most one checkpoint per 5 s idle period after a
  dirtying publication, and at least one within 10 min under continuous
  editing (`IDLE`, `MAX_AGE`). The worst case is edits spaced just over 5 s
  apart, which makes every edit a whole-dump write: about 14.7 GB/hour on g13k
  and 0.8 GB/hour on the anonymized graph. The typical case is one write per
  pause in editing. A launch with no external changes writes nothing.

## Context

GH #623: launch rebuilt every parsed page and index from the files on each
start. On g13k a cold launch takes 2.8 s to Ready; Ellis's graph took 23 s.
SPEC-storage §7.6 (Martin, 2026-10-02) fixes the remedy: a deliberately dumb
dump of the whole published generation, loaded whole and reconciled by a full
stat diff. There is no partial load, no lazy load and no incremental persisted
index.

## Decision

- **Where:** app data `launch-checkpoints/<graph-id>.bin`, keyed like the
  session and draft files (`settings::session_id`), never under the graph root.
  Unit tests keep none.
- **Format:** magic `TINECKPT` and `FORMAT` (u32), then a postcard header:
  - the lsdoc tag;
  - the canonical graph root;
  - the config revision;
  - the raw and payload lengths;
  - the payload SHA-256.

  After the header comes the zstd-compressed postcard body:
  - the published `GraphState`: parsed pages, entry list, the explicit,
    reference-candidate, alias, real-name, icon and block-ref indexes, the
    observed mtimes and content revisions;
  - the claimants and name tables;
  - every path's stamp as recorded when its bytes were read;
  - the racy set.

  `FORMAT` covers parser, config and index semantics, not the app version. A
  golden image test fails on any change to the body encoding.
- **Write:** one publisher thread per store (`tine-checkpoint`, registered in
  `tests/i21_owners.rs`). A publication that changes the generation marks it
  dirty. The thread waits for `IDLE` of quiet or `MAX_AGE` of dirtiness. It
  takes the writer briefly to capture the immutable published generation, then
  encodes and writes off the writer and UI threads. The write uses
  `atomic_file::atomic_write_with_check`: temp, fsync, rename, directory sync.
  A generation that is not Ready, not yet published, or holds unreadable files
  is skipped and retried after the next idle period. Rescan
  (`Store::rebuild_graph`) stays a forced full rebuild and requests a
  checkpoint afterwards.
- **Load:** after the current config is read, the loader validates the
  checkpoint fully before anything is served, in this order:
  1. magic and format;
  2. header;
  3. parser tag;
  4. root;
  5. config revision;
  6. lengths;
  7. checksum;
  8. decode.

  Any failure is a fallback token in diagnostics and today's full build. It is
  never a refusal or a migration. If the checkpoint is good, the loaded state
  is installed and served with readiness Loading. The launch diff then runs
  against the stored stamps and rereads every changed, new, missing or racy
  path. Ready follows when that diff completes.
- **Stale window:** page opens read the disk and take their base revision from
  it, so no save is based on checkpoint state. Graph-wide destructive operations
  wait for Ready in one of two ways:
  - Store transactions and restore take the writer. The launch holds the
    writer from serving until Ready.
  - Page rename, merge and delete find referrers through
    `tine-graph-features` `pages::refreshed_view`, which calls
    `Store::scan_refresh`; that waits for Ready before its stat diff.
  - Before a transaction takes the writer, its wait for reference
    publication reads through the crate-internal `whole_graph_reconciled`,
    which waits for Ready. The orphan-asset delete check then runs under the
    writer, where the served-but-unreconciled state cannot be observed.
  - The orphan-asset *listing* may answer from the served checkpoint. It only
    offers candidates; each delete is re-checked by `check_orphan_asset`
    under the writer, so a stale listing cannot delete a referenced asset.

  Every other graph-wide read may answer from the served checkpoint before
  Ready. The host opts in through `OpenOptions::launch_checkpoint`, a file in
  its app-data directory, never under the graph root.

## Refusals and threat scenarios

The checkpoint never refuses an operation. Each validation check falls back to
the full build:

| Check | In-scope scenario |
|---|---|
| Length or checksum | Torn or interrupted write, crash or power loss mid-write, disk error |
| Magic, format or parser tag | Another Tine build's checkpoint, after an upgrade or downgrade |
| Root | A moved or restored graph whose id collides |
| Config revision | `config.edn` edited while Tine was closed, by an external editor or a sync delivery |
| Racy stamp | An external-editor race within the timestamp granule |

## Consequences

- **Measured** (release build, median of 3, on copies of the graphs):

  | Graph | Launch | Ready | First page read | RSS after Ready |
  |---|---|---|---|---|
  | g13k | cold | 2793 ms | 197 ms | 560 MiB |
  | g13k | warm | 1037 ms (served at 950 ms) | 154 ms | 517 MiB |
  | Anonymized | cold | 136 ms | 7 ms | 28.5 MiB |
  | Anonymized | warm | 53 ms | 12 ms | 24.1 MiB |

- **Memos are not persisted.** This is the departure from §7.6. Derived-result
  memos start empty after a warm launch and refill on first use. Everything
  else in the published generation is persisted.
- **R5 is accepted.** A same-size rewrite that keeps the old mtime is not
  caught by the stat diff when the platform also reports an unchanged ctime and
  the stamp was outside the racy window. Examples are a sync client that
  preserves mtimes on a filesystem without ctime, or a rewrite inside one
  timestamp granule. That page's graph-wide answers stay stale until Rescan,
  but opening the page reads the disk. Tests:
  `checkpoint_tests::r5_an_unseen_rewrite_is_served_until_rescan_but_page_reads_see_disk`
  and `a_same_size_same_mtime_rewrite_is_caught_by_ctime`.
- **First page open waits.** Page opens take the store writer, which the launch
  diff holds. A page opened after the checkpoint is served therefore waits for
  Ready.
- **Disk use.** About 20 MB of app data for a 13k-page graph. Removing a graph
  from Tine does not delete its checkpoint.

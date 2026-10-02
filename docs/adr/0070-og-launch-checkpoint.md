# 0070. A dumb launch checkpoint serves the last published generation at launch

- **Status:** Accepted (Martin, 2026-10-02). The design is SPEC-storage §7.6;
  Martin set the persisted memos and the write cadence on the same day.
- **Date:** 2026-10-02
- **Unit cost:** no per-edit write. Each checkpoint rewrites the whole dump,
  memos and lazily built indexes included (`FORMAT` 2):
  - g13k (13,000 pages): 115.5 MiB raw (121,127,469 B), 19.6 MiB on disk
    (20,544,371 B), written in 742 ms;
  - the anonymized real graph (1,075 pages): 4.5 MiB raw (4,744,714 B),
    1.07 MiB on disk (1,124,326 B), written in 35 ms.

  These are release builds, median of 3, measured by
  `crates/tine-store/examples/checkpoint_launch_bench.rs` after one Ctrl-K
  search and one backlinks read, so the memos and indexes hold what a short
  session builds. FORMAT 1, which wrote no memos, was 20,452,145 B and
  1,109,631 B for the same state (+0.5% and +1.3%). Byte counts are identical
  across runs. Each checkpoint writes 1 file (plus its temporary sibling,
  renamed over it). Transport bytes are 0, because the checkpoint is never
  synced.

  **Frequency bound** (Martin, 2026-10-02): a checkpoint is due 60 s after the
  last dirtying publication (`IDLE`), but never sooner than 5 min after the
  previous write (`MIN_INTERVAL`); continuous editing still gets one within
  10 min of the change (`MAX_AGE`). That is at most 12 writes an hour: about
  247 MB/hour on g13k and 13.5 MB/hour on the anonymized graph in the worst
  case (bursts of edits each followed by a minute's pause). Continuous editing
  gives 6 an hour. The one exception is a store that launched cold (no
  checkpoint loaded): its first checkpoint is due 5 s after its last
  publication (`FIRST_IDLE`), so the next launch is warm; that is one extra
  write per cold launch. A launch with no external changes writes nothing.

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
  The app-data directory is Tauri's `app_data_dir` (`graph::checkpoint_app_data`,
  as `concord_ledger::attach` uses) on all five shipped targets: Linux,
  Windows, macOS, iOS and Android, with no platform branch. The first revision
  used the `dirs` crate, which has no Android arm (it resolves
  `$HOME/.local/share`, outside the app sandbox), so mobile launches kept no
  checkpoint. Checked with `cargo check -p tine -p tine-store` for
  `aarch64-linux-android` and `aarch64-apple-ios` (fake C compiler and SDK
  root; not run on a device). Unit tests keep none.
- **Format:** magic `TINECKPT` and `FORMAT` (u32), then a postcard header:
  - the lsdoc tag;
  - the canonical graph root;
  - the config key (below);
  - the raw and payload lengths;
  - the payload SHA-256.

  After the header comes the zstd-compressed postcard body:
  - the published `GraphState`: parsed pages, entry list, the explicit,
    reference-candidate, alias, real-name, icon and block-ref indexes, the
    observed mtimes and content revisions;
  - the claimants and name tables;
  - every path's stamp as recorded when its bytes were read;
  - the racy set;
  - the generation's derived half, each part in whatever state it was in:
    the lazily built block, referenced-name, alias and block-ref-count
    indexes; the query index slot (built or seeded); and the memos (the
    derived-result cache and the query memo).

  Not written: the graph-level find-entry cache, which belongs to no
  generation, and each query plan's compiled patterns, which are rebuilt from
  its filter at load. After the launch diff, the loaded memos and indexes are
  carried or dropped by exactly the rules a publication applies in memory
  (`ReadSnapshot::capture`, `carry_memos_from`); loading adds no
  invalidation rule.

  `FORMAT` covers parser, config and index semantics, not the app version. A
  golden image test fails on any change to the body encoding.
- **Config key** (Martin, 2026-10-02: config by meaning, not bytes;
  `FORMAT` 3): the SHA-256 of exactly the config fields the build and index
  path reads, by parsed value (`checkpoint::config_key`): `journals_dir`,
  `pages_dir`, `hidden`, `hidden_parse_failed_closed`,
  `block_hidden_properties`, `separated_by_commas`,
  `ignored_page_references_keywords`, `property_pages_enabled`,
  `property_pages_excludelist`, `favorites_page`,
  `journal_file_name_format`, `journal_page_title_format`,
  `preferred_format`, `file_name_format` and
  `enable_search_remove_accents`. A config edited while Tine was closed falls
  back to the full build only when it moves one of these; an edit to any other
  setting (UI, workflow, shortcuts, macros, favorites, logbook) keeps the
  checkpoint. The key is computed from the config the store opened with, which
  is the revision the launch diff compares the config file against, so a
  checkpoint loaded under it reconciles as one built under it. Completeness is
  enforced, not asserted: `config_key` destructures `Config` without `..`, so a
  new field does not compile until classified, and
  `checkpoint_config_key_tests::the_config_key_is_exactly_what_the_build_reads`
  parses tine-store and every tine-core function given a `Config` (with the
  `Config` methods they call) and fails when a field bound `_` is read there
  or a keyed field is read nowhere. The one exemption,
  `GraphMeta::from_config` (display settings returned by `Store::open`), is
  pinned by `graph_meta_stays_out_of_the_generation`.
  `checkpoint_tests::a_config_edit_while_closed_rebuilds_only_for_a_setting_the_build_reads`
  edits each of the 30 fields while closed and checks the outcome and that the
  reconciled graph equals a fresh build under the new config (byte for byte
  from a cold checkpoint, by answers from a warm one). Boundary: a value the
  app derives from config and passes in a request is part of that request,
  not of the generation.
- **Write:** one publisher thread per store (`tine-checkpoint`, registered in
  `tests/i21_owners.rs`). A publication that changes the generation marks it
  dirty. The thread waits for `IDLE` of quiet and `MIN_INTERVAL` since its
  last write, or for `MAX_AGE` of dirtiness (see the frequency bound). A store
  that launched cold waits only `FIRST_IDLE` for its first write; loading a
  checkpoint clears that (`Signal::loaded`). It takes the writer briefly to capture the immutable published generation, then
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
  5. config key;
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
| Config key | `config.edn` edited while Tine was closed in a setting the build reads, by an external editor or a sync delivery |
| Racy stamp | An external-editor race within the timestamp granule |

## Consequences

- **Measured** (release build, median of 3, on copies of the graphs):

  | Graph | Launch | Ready | First page read | RSS after Ready |
  |---|---|---|---|---|
  | g13k | cold | 2793 ms | 197 ms | 560 MiB |
  | g13k | warm | 1037 ms (served at 950 ms) | 154 ms | 517 MiB |
  | Anonymized | cold | 136 ms | 7 ms | 28.5 MiB |
  | Anonymized | warm | 53 ms | 12 ms | 24.1 MiB |

- **Memos are persisted** (Martin, 2026-10-02: reach the warm state as fast
  as possible). A backlinks read answered before the checkpoint is answered
  from the loaded memo after a warm launch: 42.4 ms → 0.1 ms on g13k, 1.8 ms →
  0.0 ms on the anonymized graph (FORMAT 1 vs 2, median of 3). A read nobody
  asked before the checkpoint costs what it costs cold.
- **Ctrl-K has no cache outside the generation.** An earlier revision of this
  ADR said block search used the find-entry cache; that was wrong. The Quick
  Switcher's search (`Store::search`, `run_graph_search`) reads only the
  published generation: page names through the page list, the alias and
  referenced-name indexes and the query index, block text through each
  block's projection (visible and folded text). The projection is written
  with its block, and the lazily built indexes are written since `FORMAT` 2,
  so a Ctrl-K asked before the checkpoint is warm after the launch: first
  Ctrl-K after a cold launch 344 ms (g13k), 377 ms (Ellis), 13.5 ms
  (anonymized); after a warm launch 127 ms, 94 ms, 7.8 ms, equal to a repeat
  of the same search in a running app (124 ms, 89 ms, 7.7 ms), which is the
  scan itself. The find-entry cache resolves a page name to its file for page
  opens; it is graph-level and not written. A lazy index built by a read does
  not dirty the generation, so a session that only reads writes no new
  checkpoint and its first Ctrl-K warms nothing for the next launch. Warm Ready did not move measurably
  (1028 ms vs 1063 ms on g13k; load 935 ms vs 970 ms).
  `checkpoint_tests::a_warm_checkpoint_loads_warm_and_answers_as_a_fresh_build`
  and the reload differential in `derived_cache_fuzz_tests` check that loaded
  memos answer as a fresh build after closed edits.
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
  from Tine (`forget_known_graph`) deletes its checkpoint, best-effort: a
  failure is ignored and never fails the removal
  (`graph::forget_launch_checkpoint`).

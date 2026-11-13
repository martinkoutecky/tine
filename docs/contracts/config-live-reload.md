# Contract — live `logseq/config.edn` (develop)

What happens when `logseq/config.edn` changes while Tine is running, and how
Tine writes it. Kept true by same-commit updates and by the tests listed at the
end, which a doc-code test checks exist. Master's contract of the same name is
the oracle for behaviour; the mechanism here is develop's.

## 1. One answer, one door

- **Answer (I-12).** `Store::config()` is the only answer to "the current
  configuration" in the backend. The frontend receives settings in `graphMeta`
  and its unreadable flag in `graphConfigUnreadable` (`src/graphConfig.ts`), and
  `applyConfigDerivedState` (src/graph.ts) is the one producer of state derived
  from it that is not read reactively (workflow, journal formats, the page
  file-name format that names Org `[[file:…]]` links in `render/pageRefs.ts`,
  favorites); graph open calls it with `null` ("apply everything"). Task workflow uses
  `applyTaskWorkflow` (src/taskWorkflow.ts) in both graph windows and Quick Capture.
  Quick Capture receives the selected graph's config with its native binding
  before activation, refreshes it on every show, and accepts live config
  publications only for that binding generation. Its scratch document remains
  separate and never becomes a graph writer.
- **Door (I-1).** Every settings write goes through
  `tine_graph_features::config::update`: a known store read problem returns
  the typed config failure without another file read. A writable attempt uses
  one read of the file, a pure text
  edit, and a `Transaction::replace` (or `create`) guarded by the revision just
  read. An outside write between the read and the commit is a conflict, and
  the edit is re-applied to the new bytes (at most four attempts). A page
  rename that moves the home page or favorites carries the same kind of
  guarded replace as its config step: one transaction after the page
  operation (`pages::commit_config`), which a title:: change that renames a
  page reuses (`pages::follow_page_identity`).
- Config writes carry no page edit kind: the Rule 8 vocabulary covers page
  content, and a Meta replace is not page content.

## 2. Taking in an outside edit

The store watcher (`crates/tine-store/src/watch.rs`) marks a batch that names
`logseq/config.edn` (case-insensitively) and reconciles it with configuration
included; poll mode (every 3 s) always includes it. The file is re-read only
when its bytes differ from the ones served (a `FileRev` byte-identity gate), so
Logseq rewriting identical bytes, or Syncthing redelivering them, costs one
read and publishes nothing. A config seen
mid-removal (metadata read, bytes already gone) is looked at again, so the
removal is published as a removal, not a modification that hides it. A removal
keeps the configuration read from the file, as Logseq's watcher does (it
ignores the unlink: `watcher_handler.cljs` `handle-changed!`): an iCloud
offload or a sync tool's delete-and-replace must not switch the graph to the
defaults, whose `:file/name-format` is `:legacy`. Only a graph that never had a
config.edn runs on the defaults; one whose config.edn is an undownloaded iCloud
placeholder (`logseq/.config.edn.icloud`) opens read-only, its settings
unknown. A real change reloads the live configuration and journal format,
and publishes an External change naming `logseq/config.edn`. The desktop
shell turns that into `graph-config-changed` with the fresh `GraphMeta`.
Both graph-open replies and live publications also carry the read problem from
`Store::config().problem`. The window adopts it only for its current binding and
graph root. One catalog notice and the page banners name config.edn, and editing
affordances use that graph restriction in addition to each page's intrinsic Org
round-trip restriction. Repeated unreadable publications do not add notices.
Repairing the config clears the notice and the graph restriction without
reopening; an unsafe Org page remains read-only. A problem-only transition
fetches no loaded pages; build-key changes retain the reconciliation below.

Own and outside config writes retain parsed pages, their observations, the
name index and every in-memory index when `checkpoint::config_key` is
unchanged. That key covers the fields read by the page build and indexes
(including file eligibility, names, journal formats and search folding),
enforced by its scanning test. A changed key still discards parsed pages,
rescans the graph files and rebuilds names and indexes before publication.
Favorites, journal templates and display settings therefore cause no file
listing, page read or parse, and their publication names no reader key: the
frontend's settings readers read `graphMeta` reactively, and
`graph-config-changed` refreshes no data (a build-key change publishes a
`full` summary instead). The comparison is with the configuration the
previous publication used, so a reload in between that moved the key is
never missed.
Config publications and frontend updates do not depend on cache invalidation.

## 3. Writing it

A setter changes only its own **top-level** key: the key is found among the
root map's direct entries, never inside a nested map (master DUP-3), and only
at a key position: a keyword that is an earlier entry's VALUE and spells the key
is not the key (`{:backup :favorites :private "keep"}` has no `:favorites`). The
readers and every setter share that one selector
(`config::find_keyword_at_map_level`). A
scalar setter replaces only the value token. Comments, unknown keys, nested
maps and spacing elsewhere survive byte for byte. A missing, empty or
comment-only file gets a new map after its comments.

Collection boundaries skip EDN character literals, including escaped delimiters
(`\}`, `\]`), named characters (`\newline`, `\space`) and Unicode characters
(`\u007D`, `\é`). The shared scanner owns this for both readers and setters:
an unrelated character must neither truncate the root before `:hidden` nor
leave part of a replaced collection behind (audit L7s1-01).

Renaming the page `:default-home` names, or a namespace parent of it, rewrites
`:default-home {:page …}` to the new name (OG `rename-page-aux`). A merge does
not (OG `merge-pages!`). A case-only rename updates `:default-home :page` to the new spelling too.

The same config step moves `:favorites` (one answerer, CM step 2 batch 4): a
favorite naming the renamed page or a namespace descendant, under any case
spelling, takes the new name in its place (OG `rename-page-aux`); a merged-away
page's favorite moves to the survivor, deduplicated (a deliberate OG difference:
OG drops it); an alias spelling stays (it still opens the page); a renamed
arrangement page moves `:tine/favorites-page`. Home and favorites are edited
independently and written together. A title:: change that renames a page moves
both the same way, as the backend observes it (`follow_page_identity`: called by
the window on an identity change, and by the CLI owner's page write). The rename
report and the follow reply carry the new list; the window adopts it and never
computes or writes favorites for a rename.

On graph open, a readable device-local session restores its tabs, histories,
order, pins and active tab before any home landing. `:default-home` supplies
the landing only without such a session, including when switching graphs.
An explicitly saved single Journals tab also wins. The `g h` command still
opens the configured home in place. This extends OG 0.10.9's `container.cljs`
`main-content`/`get-default-home-if-valid` contract to Tine's multi-tab session;
restoring a session does not request the OG `:home` route (internal card 3x4).
Session state remains outside the graph, with no additional persisted fields.

## 4. Refusals

| Situation | Behaviour | In-scope scenario |
|---|---|---|
| A delivered config is unsafe (a pages directory escaping the graph) | Watcher keeps serving the last good configuration and keeps observing pages | Sync delivery / external-editor race |
| A settings write finds a config whose first form is not a balanced map (truncated, half-saved, not a map) | `InvalidData`; the bytes are left exactly as they are | Sync delivery / external-editor race |
| An edit would produce a config without a balanced root map | `InvalidData`; nothing is written | Same: never make a malformed file out of a good one |
| A key's value has a shape Tine does not edit (`(…)`, reader tags, a collection where a scalar belongs) | `InvalidData`; nothing is written | Hand-edited or newer-Logseq config |
| A rename finds config.edn malformed, or it becomes unreadable after the graph last loaded it | The rename proceeds; home and favorites are not updated; a value the step cannot edit, or a file it cannot read, is reported as `config_skipped` and the window toasts it | Sync delivery / external-editor race; a bad config must not block page work |
| config.edn could not be read at open or on a live reload (not valid UTF-8, too large to parse, or inaccessible) | The graph runs on default settings and its editing UI is read-only: the unknown `:file/name-format` cannot safely name a page write. The transaction guard refuses with typed `Refusal::ConfigUnreadable`; a Settings config read failure carries the same internal `config-unreadable` family. The window shows `ui-failure-config-read`, never the raw read error, in the notice, page banner and Settings error. The cause remains available for diagnostics. The page host's own saves do not consult the transaction guard; this change preserves that policy. Fixing or replacing config.edn lifts the graph restriction and clears the notice through the watcher without a restart; intrinsic Org restrictions remain | Sync delivery; external editor; disk/permission error; an undownloaded or corrupted config |
| A rename's config.edn write fails after its page files were published (disk full, permission, a lost race) | The pages stay renamed and nothing is rolled back; config.edn keeps its old bytes. The app's rename returns the rename as done with the cause in `config_skipped`, so the window refreshes and toasts that the page was renamed but home and favorites may still name the old page; the CLI keeps the error and reports the rename as uncertain (`IncompleteRename`) | Disk error between the last page step and the config step |

A malformed config never blocks opening the graph: the reader is lenient and
pages open as usual.

## 5. What this does not close

`:favorites` is written wholesale from the list the frontend holds. Live
reload shrinks, but does not close, the window in which a favorite added
elsewhere is overwritten by the next star toggled in Tine (master §6).

A crash between a rename's last page step and its config step leaves home and
favorites naming the old page; the user re-picks them. No page content is at
risk. A title:: change made while no Tine process observes it (an outside
editor with the app and `tine serve` closed) is not followed: the next open
lists the favorite under its old name, as OG never follows title:: edits.

## Tests

- src/components/Page.configUnreadable.test.tsx
- crates/tine-graph-features/tests/config_writes.rs::unreadable_settings_use_the_config_refusal_family_without_changing_bytes
- crates/tine-store/src/store/page_read.rs::graph_config_restriction_does_not_erase_the_pages_intrinsic_read_only_reason
- src-tauri/src/watcher.rs::live_config_publications_carry_the_read_problem_and_its_recovery
- crates/tine-graph-features/tests/audit_rn_b.rs::audit_l7s1_01_character_literals_keep_hidden_and_surgical_config_edits
- crates/tine-store/tests/config_live_reload.rs::an_outside_config_edit_is_taken_in_by_the_notify_watcher
- src/capture.workflow.test.tsx
- crates/tine-store/tests/config_live_reload.rs::an_outside_config_edit_is_taken_in_by_the_poll_watcher
- crates/tine-store/tests/config_live_reload.rs::an_identical_config_rewrite_publishes_nothing
- crates/tine-store/tests/config_live_reload.rs::settings_only_writes_keep_parsed_pages
- crates/tine-store/tests/config_live_reload.rs::outside_settings_keep_parsed_pages_but_build_config_reparses
- crates/tine-store/tests/config_live_reload.rs::own_build_config_change_still_reparses_pages
- crates/tine-store/tests/og_perf_publish.rs::creates_and_a_favourite_toggle_list_and_open_no_other_page
- crates/tine-store/tests/og_perf_publish.rs::an_outside_favourite_edit_rescans_nothing_but_a_hidden_edit_does
- crates/tine-store/tests/config_live_reload.rs::an_unsafe_delivered_config_keeps_the_served_one_and_pages_stay_observed
- crates/tine-store/tests/config_live_reload.rs::a_removed_config_keeps_its_settings_until_it_returns
- crates/tine-store/tests/config_live_reload.rs::an_undownloaded_icloud_config_opens_the_graph_read_only
- crates/tine-graph-features/tests/config_writes.rs::every_setter_edits_the_top_level_key_never_a_nested_shadow
- crates/tine-graph-features/tests/config_writes.rs::every_setter_leaves_a_keyword_value_that_spells_its_key_alone
- crates/tine-graph-features/tests/config_writes.rs::a_real_key_after_a_keyword_value_is_the_one_replaced
- crates/tine-graph-features/tests/config_writes.rs::every_setter_refuses_a_config_that_is_not_a_balanced_map
- crates/tine-graph-features/tests/config_writes.rs::a_malformed_config_never_blocks_open
- crates/tine-graph-features/tests/config_writes.rs::a_config_write_killed_after_its_step_reopens_whole
- crates/tine-graph-features/tests/config_writes.rs::every_config_setter_goes_through_the_one_guarded_update
- crates/tine-graph-features/tests/client.rs::config_retry_reapplies_edit_over_external_write
- crates/tine-graph-features/tests/home_rename.rs::renaming_the_home_page_moves_default_home_with_it
- crates/tine-graph-features/tests/home_rename.rs::merging_the_home_page_into_another_keeps_default_home
- crates/tine-graph-features/tests/home_rename.rs::a_malformed_config_neither_blocks_the_rename_nor_is_rewritten
- crates/tine-graph-features/tests/home_rename.rs::a_non_utf8_config_refuses_the_rename_before_anything_changes
- crates/tine-store/tests/config_live_reload.rs::a_write_while_config_is_unreadable_is_the_typed_config_refusal
- src-tauri/src/commands/rename_tests.rs::a_rename_refused_for_an_unreadable_config_reaches_the_window_as_its_wire_family
- src/backend.configUnreadable.test.ts
- crates/tine-graph-features/tests/home_rename.rs::a_failed_config_write_after_the_page_files_moved_reports_the_rename
- src/document/graphRewrite.test.ts
- crates/tine-graph-features/tests/home_rename.rs::a_home_rename_killed_at_each_step_reopens_whole
- crates/tine-graph-features/tests/home_rename.rs::home_and_favorites_move_together_in_the_config_step
- crates/tine-graph-features/tests/home_rename.rs::a_favorite_renamed_onto_another_favorite_is_deduplicated
- crates/tine-graph-features/tests/home_rename.rs::home_and_favorites_follow_independently
- crates/tine-graph-features/tests/cm_favorites_rename.rs::favorites_after_rename_through_the_rust_entry
- src-tauri/src/owner/owner_tests/cm_favorites_tests.rs::favorites_after_a_headless_cli_rename
- src/cmFavoritesRename.test.ts

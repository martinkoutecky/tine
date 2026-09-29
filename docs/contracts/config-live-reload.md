# Contract — live `logseq/config.edn` (og)

What happens when `logseq/config.edn` changes while Tine is running, and how
Tine writes it. Kept true by same-commit updates and by the tests listed at the
end, which a doc-code test checks exist. Master's contract of the same name is
the oracle for behaviour; the mechanism here is og's.

## 1. One answer, one door

- **Answer (I-12).** `Store::config()` is the only answer to "the current
  configuration" in the backend. The frontend's is the `graphMeta` signal, and
  `applyConfigDerivedState` (src/graph.ts) is the one producer of state derived
  from it that is not read reactively (workflow, journal title format,
  favorites); graph open calls it with `null` ("apply everything").
- **Door (I-1).** Every settings write goes through
  `tine_graph_features::config::update`: one read of the file, a pure text
  edit, and a `Transaction::replace` (or `create`) guarded by the revision just
  read. An outside write between the read and the commit is a conflict, and
  the edit is re-applied to the new bytes (at most four attempts). A page
  rename that moves the home page carries the same kind of guarded replace as
  the last step of its own `RenamePage` transaction.
- Config writes carry no page edit kind: the Rule 8 vocabulary covers page
  content, and a Meta replace is not page content.

## 2. Taking in an outside edit

The store watcher (`crates/tine-store/src/watch.rs`) marks a batch that names
`logseq/config.edn` (case-insensitively) and reconciles it with configuration
included; poll mode (every 3 s) always includes it. The file is re-read only
when its bytes differ from the ones served (a `FileRev` byte-identity gate), so
Logseq rewriting identical bytes, or Syncthing redelivering them, costs one
read and publishes nothing. A real change reloads the configuration, discards
parsed pages (the pages directory or name format may have moved) and publishes
an External change naming `logseq/config.edn`. The desktop shell turns that
into `graph-config-changed` with the fresh `GraphMeta`.

Master 0.6.985 also avoided re-indexing on a settings-only change. That half is
X-class (SQLite index); og has no index to spare, and the byte-identity gate
plus "reload only when the bytes changed" is its replacement.

## 3. Writing it

A setter changes only its own **top-level** key: the key is found among the
root map's direct entries, never inside a nested map (master DUP-3), and a
scalar setter replaces only the value token. Comments, unknown keys, nested
maps and spacing elsewhere survive byte for byte. A missing, empty or
comment-only file gets a new map after its comments.

Renaming the page `:default-home` names, or a namespace parent of it, rewrites
`:default-home {:page …}` to the new name (OG `rename-page-aux`). A merge does
not (OG `merge-pages!`). A case-only rename writes nothing, so home keeps its
old spelling, which still resolves to the page.

## 4. Refusals

| Situation | Behaviour | In-scope scenario |
|---|---|---|
| A delivered config is unsafe (a pages directory escaping the graph) | Watcher keeps serving the last good configuration and keeps observing pages | Sync delivery / external-editor race |
| A settings write finds a config whose first form is not a balanced map (truncated, half-saved, not a map) | `InvalidData`; the bytes are left exactly as they are | Sync delivery / external-editor race |
| An edit would produce a config without a balanced root map | `InvalidData`; nothing is written | Same: never make a malformed file out of a good one |
| A key's value has a shape Tine does not edit (`(…)`, reader tags, a collection where a scalar belongs) | `InvalidData`; nothing is written | Hand-edited or newer-Logseq config |
| A rename finds config.edn unreadable or malformed | The rename proceeds; home is not updated | Sync delivery / external-editor race; a bad config must not block page work |

A malformed config never blocks opening the graph: the reader is lenient and
pages open as usual.

## 5. What this does not close

`:favorites` is written wholesale from the list the frontend holds. Live
reload shrinks, but does not close, the window in which a favorite added
elsewhere is overwritten by the next star toggled in Tine (master §6).

A crash between a rename's last page step and its config step leaves home
naming the old page; the user re-picks it in Settings. No page content is at
risk.

## Tests

- crates/tine-store/tests/config_live_reload.rs::an_outside_config_edit_is_taken_in_by_the_notify_watcher
- crates/tine-store/tests/config_live_reload.rs::an_outside_config_edit_is_taken_in_by_the_poll_watcher
- crates/tine-store/tests/config_live_reload.rs::an_identical_config_rewrite_publishes_nothing
- crates/tine-store/tests/config_live_reload.rs::an_unsafe_delivered_config_keeps_the_served_one_and_pages_stay_observed
- crates/tine-store/tests/config_live_reload.rs::a_removed_then_restored_config_is_followed
- crates/tine-graph-features/tests/config_writes.rs::every_setter_edits_the_top_level_key_never_a_nested_shadow
- crates/tine-graph-features/tests/config_writes.rs::every_setter_refuses_a_config_that_is_not_a_balanced_map
- crates/tine-graph-features/tests/config_writes.rs::a_malformed_config_never_blocks_open
- crates/tine-graph-features/tests/config_writes.rs::a_config_write_killed_after_its_step_reopens_whole
- crates/tine-graph-features/tests/config_writes.rs::every_config_setter_goes_through_the_one_guarded_update
- crates/tine-graph-features/tests/client.rs::config_retry_reapplies_edit_over_external_write
- crates/tine-graph-features/tests/home_rename.rs::renaming_the_home_page_moves_default_home_with_it
- crates/tine-graph-features/tests/home_rename.rs::merging_the_home_page_into_another_keeps_default_home
- crates/tine-graph-features/tests/home_rename.rs::a_malformed_config_neither_blocks_the_rename_nor_is_rewritten
- crates/tine-graph-features/tests/home_rename.rs::a_home_rename_killed_at_each_step_reopens_whole

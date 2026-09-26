# og plain-file storage contract

The graph files are the durable record. A save compares the caller's revision with
the current file, then atomically publishes whole bytes. Another local writer can
replace that file between the final comparison and rename; mandatory cross-process
locks are outside this plain-file contract. The editor retains unsaved content
on every refusal and offers conflict resolution or retry.

`Store::is_graph_ready()` reports initial graph loading without waiting.
`Ok(false)` means graph-wide answers can still block. After `Err(Failed(reason))`,
`page()` can read and parse an existing file, but saves and observed edits do
not publish a graph generation. `whole_graph()` returns the load error. A
successful `scan_refresh()` retries the load and publishes a fresh generation;
the answer becomes `Ok(true)`. `Err(Closed)` is terminal for that store.

## I-8 refusal scenarios

Each row below is keyed by the source file, owning function and refusal family.
The count is the number of production constructions or save-adapter outcomes;
the source guard fails when an unreviewed site appears. These scenarios involve
ordinary sync, external editors, user actions, malformed files, or graph lifecycle.

| Key | Count | In-scope scenario and required response |
|---|---:|---|
| `transaction.rs::path::InvalidTarget` | 3 | A sync update changes a path into a symlink or invalid area while an operation resolves it; refuse access outside the approved graph root. |
| `transaction.rs::twin::Twin` | 1 | Sync introduces a second physical file for the same page name or journal day; refuse a write that could choose the wrong one. |
| `transaction.rs::preflight::InvalidTarget` | 11 | A legitimate caller supplies a malformed path or unsupported operation (including moving config), or an external rename changes target shape; refuse before any step changes disk. Config moves require live config publication. |
| `transaction.rs::preflight::Twin` | 1 | A sync-created same-name page appears between page creation and commit; refuse the ambiguous create. |
| `transaction.rs::preflight::ReadOnly` | 1 | An Org page no longer round-trips after an external edit; keep its bytes and the unsaved editor proposal. |
| `transaction.rs::preflight::Undecodable` | 2 | A pasted or imported page stream is invalid UTF-8; refuse before making an unreadable page. |
| `transaction.rs::preflight::RepeatedFile` | 1 | Two steps of one user operation choose the same unique filename; refuse the ambiguous plan. |
| `transaction.rs::apply::InvalidTarget` | 1 | A generated unique candidate is no longer a valid target after a concurrent change; refuse that candidate. |
| `transaction.rs::apply::RepeatedFile` | 1 | A unique candidate collides with another step after planning; refuse rather than overwrite. |
| `transaction.rs::apply::Undecodable` | 2 | Imported bytes fail UTF-8 validation after staging; remove the stage and keep the original. |
| `transaction.rs::commit::Closed` | 1 | A graph is closed while a queued save waits; refuse its old binding. |
| `transaction.rs::commit::RepeatedFile` | 1 | A multi-step action names one file twice; refuse before any write. |
| `transaction.rs::rewrite::Undecodable` | 1 | Sync makes a referrer invalid UTF-8 before rename rewrite; keep that file and refuse the rename. |
| `transaction.rs::rewrite::ReadOnly` | 1 | An Org referrer is not round-trip editable; keep its bytes instead of rewriting it. |
| `transaction.rs::validate_stream::Undecodable` | 2 | A streamed page import contains invalid UTF-8; refuse before publication. |
| `store.rs::save::Closed` | 2 | A queued editor save arrives after graph close; return the closed family without writing. |
| `store.rs::save::Conflict` | 2 | An external edit or alias change makes the caller's revision or file identity stale; return current disk data for resolution. |
| `store.rs::save::Deleted` | 1 | Sync deletes a page while its editor buffer is open; retain the buffer and report deletion. |
| `store.rs::save::ReadOnly` | 1 | A parser or format check rejects a round-trip edit; retain the buffer. |
| `store.rs::save::Twin` | 1 | A second same-name physical file appears; refuse ambiguous publication. |
| `store.rs::save::InvalidTarget` | 2 | A saved target becomes invalid or a transaction reports a target refusal; retain the buffer. |
| `store.rs::save::GuideEphemeral` | 1 | The user edits a bundled Guide page that has no graph file; refuse persistence of the virtual copy and prompt an explicit graph copy. |

The following rows cover refusals outside the two constructor families counted
above. Their individual call sites are inventoried in `og/batches/E-survey-errors.md`
§I-8; the key here is the production owner and operation family. Error-return
sites that report a disk failure rather than refusing an operation are covered
by I-9's typed failure paths.

| Owner / operation family | In-scope scenario and required response |
|---|---|
| `tine-store::model` path and graph acquisition | A configured graph or asset directory is retargeted, malformed, or resolves outside the approved root after sync or external editing; refuse reads and writes through that path. |
| `tine-store::store` read, scan and handoff | Graph close revokes queued requests; a symlink, non-file, or escaped path appears after the caller selected it; refuse stale bytes and OS handoff. A failed initial parse withholds an unpublished view. |
| `tine-store::watch` reconcile | A graph closes, its root disappears, or an external config edit changes directory layout; stop reconciliation rather than publishing a false view. |
| `tine-store::restore` source, destination and recovery | A selected snapshot source changes type or length, a live/recovery path is retargeted, or an external writer creates the destination; refuse publication and retain displaced bytes in recovery. |
| `tine-store::publish` staged site | An external writer retargets output or stage paths or wins the destination name; refuse replacement and retain the previous site. |
| `tine-graph-features::pages` rename, rescue, merge and delete | Sync or an external editor changes a revision, creates a twin, occupies a destination, or makes Org non-round-tripping; refuse the affected transaction and retain source bytes. |
| `tine-graph-features::conflicts` resolve | A sync conflict winner or copy changes or disappears during resolution, or an Org member is not editable; preserve both sides and require retry. |
| `tine-graph-features::journals` migration and trash | A journal disappears or changes after selection; refuse that item and leave other journals intact. |
| `tine-graph-features::pdf` highlight and sidecar | Sidecar or notes bytes change concurrently, a malformed imported sidecar appears, or Org notes cannot round-trip; retain the source and refuse or retry within the bounded loop. |
| `tine-graph-features::config` and `assets` | Config or an asset changes repeatedly while applying a user update; stop before overwriting the external winner. |
| `src-tauri::backup` restore selection | A backup is incomplete, belongs to another graph, fails its manifest hash, loses a source file, or changes during verification; refuse restore before touching live content. A failed pre-restore safety snapshot also refuses publication. |
| `src-tauri::state` graph binding | Two windows try to own overlapping roots, or a queued command carries an old binding generation; refuse a wrong-graph write. |
| `src::persistence` frontend save gate | A page is tombstoned, conflicted, held as the source of a cross-page move, or the graph switch still has pending writes; retain the editor buffer and refuse the unsafe completion. |

Review rule: a new refusal must identify a reachable scenario involving an honest
local user, sync or external editor. Source scans cannot prove reachability;
the reviewer traces the path and records the scenario here before accepting it.

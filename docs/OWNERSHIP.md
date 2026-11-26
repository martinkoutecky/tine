# Who owns what

Each question has one owner in the code; details are in each file's header.

## Editing

- **The live edit tree** is the frontend's document module, `src/document/`.
  `src/document/index.ts` is its only door: production code outside the module
  imports from there. An edit is an intent exported there (`setRaw`,
  `splitBlock`, `moveBlock`, ...) that changes the model and marks the page dirty.
- **What an outline edit does to a page** is the outline kernel's answer
  (`crates/tine-outline/src/outline.rs`), applied by `src/document/kernel.ts`.

## Acceptance

- **Whether a page edit, move, rename or deletion is taken** is decided by the
  page host, `crates/tine-store/src/page_host/`, one per graph binding, started by
  `start_host` in `src-tauri/src/graph.rs`. It is the only writer of page edits
  and owns base revisions, conflicts and crash drafts.
- **The window's side** is `src/document/host/`: `wiring.ts` is the only module
  that issues `page_*` commands, and `client.ts` keeps no base revisions,
  retries, drafts or conflict detection of its own.

## Durable publication

- **Writing bytes to disk** is `PreparedWrite` in
  `crates/tine-store/src/atomic_file.rs`: temporary file, fsync, the caller's
  base-revision check, rename, directory fsync. The page host uses it for pages;
  `crates/tine-store/src/transaction.rs` uses it for other files and plans the
  multi-page writes the host then performs.
- **Telling the windows** what changed is the Store's publication (rules in
  [`storage-contract.md`](storage-contract.md)), applied by `src/document/external.ts`.

## Page identity

- **Which file a page name opens** is `NameOwners::opens` (`crates/tine-store/src/store.rs`),
  fed by `crates/tine-store/src/store/name_owners.rs`.
- **The frontend** only looks names up in the latest page inventory,
  `src/pageIndex.ts`, keyed by `pageIdentityKey` (`src/pageIdentity.ts`, the
  same key the core computes); it decides no precedence itself.

## Graph lifetime

- **Opening and binding a graph to a window** is `load_graph` in
  `src-tauri/src/graph.rs`; closing it is `close_graph_window` in
  `src-tauri/src/commands.rs`. Each binding has a `binding_generation`
  (`src-tauri/src/state.rs`) that frontend mutations carry, so one queued before
  a graph switch cannot run against the new graph.
- **On the frontend**, `src/graph.ts` switches graphs, `src/binding.ts`
  captures the binding, and `src/owned.ts` makes a retired owner's late result
  `stale`, never applied.

## Disposal

- **Frontend state** is released by `forgetPage` and `resetStore` in
  `src/document/workingSet.ts`; `resetStore` calls `invalidateBinding`
  (`src/binding.ts`), which runs every registered clear ([contract](contracts/frontend-memory.md)).
- **A page host no window owns** is stopped by `src-tauri/src/host_retirement.rs`,
  or adopted when the same graph is opened again.

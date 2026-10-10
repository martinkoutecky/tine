/** The document module: the frontend's loaded pages, every edit to them, undo,
 * and their persistence. This file is the only door; production code outside
 * src/document imports from here (guard: boundary.guard.test.ts).
 * SURFACE.txt lists every export; new exports after batch 02b need a `# why:` justification.
 *
 * Reading. `existingBlockId(raw, format, facts?)` reads authored identity from
 * an exact buffer; editor facts take precedence and mismatched facts throw.
 * O(block bytes): loaded parser-owned absence avoids a parse; unknown/possible
 * identity uses cached parser regions. Callers need no knowledge of the seeds.
 * `node(id)`, `childIds`, `pageRoots`, `loadedPage`, `feedNames`,
 * `isLoaded` read the live Solid store on each call. Call them inside a tracking
 * scope (JSX, memo, effect) to re-render on change; a value read once in setup
 * is a snapshot, not a subscription. Returned values are readonly.
 * `loadRoutedPage` admits the first routed graph page and enables saves even
 * when no journal feed has loaded; O(blocks of that page), and an unsafe
 * replacement leaves the current edit intact. Satellite/scratch loads use
 * `ensurePageLoaded` and cannot enable persistence.
 *
 * `selectionMarkdown(includeSubtree?)` returns public clipboard Markdown in
 * selected-root order, honoring the copy preference unless overridden. Cuts
 * pass true to include every removed descendant. O(visible-order resolution + selected subtree bytes);
 * an empty selection returns an empty string; no save or undo side effects.
 *
 * Editing. Change a page only through an intent exported here (`setRaw`,
 * `splitBlock`, `moveBlock`, `setBlockProperty`, ...). An intent updates the
 * model and marks the page dirty; user edits also record undo (`ensureBlockId`
 * stamps an id without undo). `withUndoUnit` groups the synchronous mutations
 * of its callback into one undo step; it does not span awaits. A new intent
 * lives in the matching `edits/*.ts` file. In-memory-only changes (no save, no undo):
 * `revealNode` (expand for find) and the page-header edit begin/finish pair.
 * `sanitizeOutlineIdsForPaste` prepares an ID-bearing ordinary paste by checking
 * loaded blocks and backend ID lookups in chunks of at most 128 IDs. Each lookup
 * can scan graph pages in memory and read pages for IDs it cannot place; paid per explicit paste
 * that carries IDs, never per keystroke or save;
 * it returns null when the target graph changes and strips uncertain IDs on a
 * lookup failure. The caller needs no knowledge of which pages are loaded.
 * `blockPositionRef` and `settleBlockRef` let a saved session name an ID-less
 * zoomed block by position: navigation never writes an `id::`. `persistBlockRefTarget`
 * stamps the target ID and edits the source only once the target's published
 * bytes carry that ID (STEP3 §8, Q2). A crash between the two may leave an
 * unreferenced ID, never a dangling reference. Failure returns null or false.
 * Cost is one target save, plus a page lookup when the target is not loaded.
 * `ensurePagePropertyOnKeyPage` writes a property declaration on its normalized
 * key page: one page read, then one guarded page edit. It rejects
 * graph changes, conflicts and read-only pages; callers report the error.
 *
 * Saving. The graph's page host (tine-store `page_host`) owns every page
 * write; `host/wiring.ts` is this window's client of it and the only caller of
 * the `page_*` commands (I-1). Pages are sent as whole-page snapshots, never
 * as operations. `markDirty` requires an edit kind; the client sends the page
 * after a short debounce on the version its text was installed at, and the
 * host saves it, keeps a crash-recovery draft until it is saved, and answers.
 * Input on a page whose file changed meanwhile becomes a conflict the host
 * reports; `resolveConflict` sends Keep mine on the disk state the conflict
 * showed, or takes the host's text for Use disk. A block entering editing
 * opens its page in the host (`bindHost` binds the window after a graph load).
 * Multi-page intents that move blocks between pages run `persistTransfer`: the
 * pages freeze, their input drains, and one host move per source carries the
 * blocks; a crash between moves can leave a block in both pages, never in
 * neither (§8). Other multi-page intents are independent page edits.
 * `flushPage` / `flushAll` wait until the host published the input (`settle`).
 * An incomplete page-header draft remains unsent without an autosave toast;
 * exiting that editor reports invalid syntax once. Creating a page file goes
 * through `createPage`, which refuses locally with a typed `CreatePageRefusal`,
 * distinct from a disk conflict (an existing file). Kinds are O(1) bookkeeping
 * per edit and no disk bytes.
 *
 * Outside changes. A page the host holds for this window takes disk changes
 * as host mail: its text when it is clean and unheld, a reported conflict over
 * input. `applyGraphChange` handles one watcher event for other pages using
 * `reloadDisposition`: an own-save echo keeps content and undo; a clean page
 * reloads, unless it is being edited or moved (then the change is deferred);
 * a clean removed page that is not being edited leaves its route;
 * results that land after a graph switch are dropped (I-20). This module never imports the router: route
 * and feed effects go through handlers the app installs
 * (`installExternalChangeUiHandler`, `installAliasDraftRouteHandler`,
 * `installHistoryRouteContextAdapter`). An effective title change uses the
 * exact physical path to rekey navigation, loaded page ownership and its save
 * baseline in that order. The UI installs `installPageIdentityNavigation`;
 * rekeying refuses a name collision or an unsafe external reload.
 * `applyGraphChangesBulk` applies a checkout-sized batch with one revision
 * bump, at most one feed restart and one summary toast. With "always ask"
 * (conflictPolicy.ts) the one silent case, a loaded clean page, is held for its
 * bar instead; `replayDeferredExternalReloads` is the focus-return sweep. */
export { blockIsGridView, collapseEpochOf, node, childIds, pageRoots, loadedPage, feedNames, isLoaded, formatForBlock, formatForPage, mainPages, pageByName } from "./model";
export type { ReadonlyFeedPage as FeedPage, ReadonlyNode as Node } from "./model";
export { trackAssetWrite } from "./assetWrites";
export { applyLiveResolution, bindHost, conflictReason, conflicts, consumedAnswer, createPage, CreatePageRefusal, flushAll, flushPage, installAliasDraftRouteHandler, isConflicted, liveConflictDraft, isDirty, isSaving, markDirty, refuseConflictedMove, resolveConflict, sameLiveDraft, unsavedDrafts, unsavedPageCount, type UnsavedState } from "./host/wiring";
export { pendingDataRevision } from "./host/wiring";
export type { DiskToken, DraftStatus, OwedPage, PageMail, PageOperation, PageRefusal, PublishedNeed } from "./host/protocol";
export { applyGraphChange, applyGraphChangesBulk, installExternalChangeUiHandler } from "./external";
export { replayDeferredExternalReloads, whenPageReplaceable } from "./deferredReload";
export { admitPageFile, appendFeed, deletePage, ensurePageLoaded, loadFeed, loadGuidePages, loadRoutedPage, pageLoadRefusalMessage, pinPageWhileDrafting, registerPaneRouteProvider, reloadHlsIfLoaded, reportPageLoadRefusal, resetStore, restoreTodayJournalInFeed, type PageLoadRefusal } from "./workingSet";
export { installRenameRefreshHandler, renamePageOnDisk } from "./graphRewrite";
export { graphRewriteFrozen, tryFreezeGraphRewrite } from "./graphRewriteState";
export { emptyPage, favoritesArrangementPage, favoritesArrangementBlocks, resolveGuideBlockRef, resolveGuidePageDto, withToday, toLoadablePage, carryTodayPage, captureScratchPage, journalTemplatePage, demoJournalPage, switcherPage, queryWorkspacePage } from "./convert";
export { depthOf, nextVisible, pageVisibleOrder, prevVisible, visibleOrder } from "./tree";
export type { OutlineScope } from "./tree";
export { installHistoryRouteContextAdapter, redo, toggleUndoRedoMode, undo, undoTopTag, withUndoUnit } from "./history";
export type { HistoryRouteContext } from "./history";
export { deleteBlock, ensureEmptyBlock, indentBlock, insertEmptyChildBlock, insertOutlineAfter, insertOutlineBefore, insertOutlineChildren, mergeWithNext, mergeWithPrev, outdentBlock, outlineFits, replaceChildOrders, replaceEmptyBlockWithOutline, revealNode, setCollapsed, setRaw, splitBlock, toggleCollapse } from "./edits/blocks";
export { pasteClipboardPayload, sanitizeOutlineIdsForPaste } from "./edits/paste";
export { appendToTodayJournal, captureToPage } from "./edits/capture";
export { beginPageHeaderEdit, blockPageReadOnly, blockProperty, blockWritable, collapsibleDescendantIds, expandAncestors, finishPageHeaderEdit, makeOwnNumberedList, orderedListMarker, pageHeaderProperties, promotePagePreamble, readPageProperties, readPageProperty, readSchedule, removeOwnNumberedList, setBlockProperty, setCollapsedDeep, setCollapsedDescendants, setHeading, setPageProperty, setSchedule, stopOwnNumberedListOnEmptyEnter, toggleBlockProperty, toggleListItemAtIndex, toggleOwnNumberedList } from "./edits/properties";
export { ensurePagePropertyOnKeyPage } from "./edits/propertyDeclaration";
export { blockExternalId, blockPositionRef, blockRef, ensureBlockId, existingBlockId, isBlockRefUuid, persistBlockRefTarget, resolveBlockRef, settleBlockRef } from "./edits/identity";
export { blockSubtreeMarkdown, buildClipboardPayload, dtoSubtreeMarkdown, exportNodesFor } from "./edits/serialize";
export { clearSelection, cycleSelectionTasks, deleteSelection, expandBlockSelection, extendSelectionTo, hasSelection, indentSelection, isSelected, moveSelection, outdentSelection, selectBlock, selectBlockSubtree, selectedIds, selectionMarkdown, setSelectionHeading } from "./edits/selection";
export { extendFeedForScroll, isBlockMoving, moveBlock, moveBlocksRelative, moveBlockFeed, moveItem, moveSelectionItems, nextVisibleOrExtend, setFeedExtender, withBlockMoving } from "./edits/moves";
export { installPageIdentityNavigation, rekeyPageIdentityByPath } from "./workingSet";
export { carryUnfinished } from "./edits/carry";

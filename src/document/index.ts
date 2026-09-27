/** The document module: the frontend's loaded pages, every edit to them, undo,
 * and their persistence. This file is the only door; production code outside
 * src/document imports from here (guard: boundary.guard.test.ts).
 *
 * Reading. `node(id)`, `childIds`, `pageRoots`, `loadedPage`, `feedNames`,
 * `isLoaded` read the live Solid store on each call. Call them inside a tracking
 * scope (JSX, memo, effect) to re-render on change; a value read once in setup
 * is a snapshot, not a subscription. Returned values are readonly.
 *
 * Editing. Change a page only through an intent exported here (`setRaw`,
 * `splitBlock`, `moveBlock`, `setBlockProperty`, ...). An intent updates the
 * model and marks the page dirty; user edits also record undo (`ensureBlockId`
 * stamps an id without undo). `withUndoUnit` groups the synchronous mutations
 * of its callback into one undo step; it does not span awaits. A new intent
 * lives in the matching `edits/*.ts` file. Cross-page moves do not mark the
 * source dirty directly: see Saving. In-memory-only changes (no save, no undo):
 * `revealNode` (expand for find) and the page-header edit begin/finish pair.
 *
 * Saving. Pages save as whole-page snapshots, never as operations: `markDirty`
 * schedules a debounced (400 ms) save of every dirty page; each page saves
 * serially against the file revision it last read, so a file changed on disk
 * becomes a conflict (`conflicts`), never an overwrite. The conflict bar
 * resolves it with `forceSave` ("keep mine", then `clearConflict`),
 * `reloadPage`, or `forgetPage` when the file is gone. `flushPage` / `flushAll`
 * wait for saves and return false when a page is held, conflicted or failed
 * (`flushAll` drains a bounded number of passes). A move across pages holds the
 * source page's save until the destination's save has landed (`persistCrossPage`
 * / `releaseSourcesFor`; follow it for any new cross-page intent, undo/redo
 * included), so a crash duplicates a block and never loses it (I-3 backlog:
 * one request per multi-page intent is a later batch). Creating a page file
 * goes through `createPage`, which refuses locally with a typed
 * `CreatePageRefusal`, distinct from a disk conflict. Only save/engine.ts calls
 * the backend's savePage/deletePage (I-1).
 *
 * Outside changes. `applyGraphChange` handles one watcher event using
 * `reloadDisposition`: an own-save echo keeps content and undo; a clean page
 * reloads, unless it is being edited or moved (then the change is skipped); a
 * dirty, saving or conflicted page becomes a conflict, including when its file
 * was removed; a clean removed page that is not being edited leaves its route;
 * results that land after a graph switch are dropped (I-20). This module never imports the router: route
 * and feed effects go through handlers the app installs
 * (`installExternalChangeUiHandler`, `installAliasDraftRouteHandler`,
 * `installHistoryRouteContextAdapter`). */
export { blockIsGridView, node, childIds, pageRoots, loadedPage, feedNames, isLoaded, formatForBlock, formatForPage, mainPages, pageByName } from "./model";
export type { ReadonlyFeedPage as FeedPage, ReadonlyNode as Node } from "./model";
export { clearConflict, conflicts, createPage, CreatePageRefusal, flushAll, flushPage, forceSave, installAliasDraftRouteHandler, isConflicted, isDirty, isSaving, markDirty, trackAssetWrite } from "./save/engine";
export { applyGraphChange, installExternalChangeUiHandler } from "./external";
export { appendFeed, deletePage, ensurePageLoaded, forgetPage, loadFeed, loadGuidePages, registerPaneRouteProvider, reloadHlsIfLoaded, reloadPage, resetStore, restoreTodayJournalInFeed } from "./workingSet";
export { emptyPage, resolveGuideBlockRef, resolveGuidePageDto, withToday, toLoadablePage, carryTodayPage, captureScratchPage, journalTemplatePage, demoJournalPage, switcherPage, queryWorkspacePage } from "./convert";
export { depthOf, nextVisible, pageVisibleOrder, prevVisible, visibleOrder } from "./tree";
export type { OutlineScope } from "./tree";
export { installHistoryRouteContextAdapter, redo, toggleUndoRedoMode, undo, withUndoUnit } from "./history";
export type { HistoryRouteContext } from "./history";
export { deleteBlock, ensureEmptyBlock, indentBlock, insertEmptyChildBlock, insertOutlineAfter, insertOutlineChildren, mergeWithPrev, outdentBlock, replaceChildOrders, replaceEmptyBlockWithOutline, revealNode, setCollapsed, setRaw, splitBlock, toggleCollapse } from "./edits/blocks";
export { pasteClipboardPayload } from "./edits/paste";
export { appendToTodayJournal, captureToPage } from "./edits/capture";
export { beginPageHeaderEdit, blockPageReadOnly, blockProperty, blockWritable, collapsibleDescendantIds, finishPageHeaderEdit, makeOwnNumberedList, orderedListMarker, promotePagePreamble, readPageProperty, readSchedule, removeOwnNumberedList, setBlockProperty, setCollapsedDeep, setCollapsedDescendants, setHeading, setPageProperty, setSchedule, stopOwnNumberedListOnEmptyEnter, toggleBlockProperty, toggleListItemAtIndex, toggleOwnNumberedList } from "./edits/properties";
export { blockExternalId, blockRef, ensureBlockId, persistBlockRefTarget, persistentBlockRef, resolveBlockRef } from "./edits/identity";
export { blockSubtreeMarkdown, buildClipboardPayload, dtoSubtreeMarkdown, exportNodesFor } from "./edits/serialize";
export { clearSelection, cycleSelectionTasks, deleteSelection, extendSelectionTo, hasSelection, indentSelection, isSelected, moveSelection, outdentSelection, selectBlock, selectedIds, selectionMarkdown } from "./edits/selection";
export { extendFeedForScroll, isBlockMoving, moveBlock, moveBlockFeed, moveItem, moveSelectionItems, nextVisibleOrExtend, prepareCrossPageSources, setBlockMoving, setFeedExtender } from "./edits/moves";
export { carryUnfinished } from "./edits/carry";

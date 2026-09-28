/** The document module: the frontend's loaded pages, every edit to them, undo,
 * and their persistence. This file is the only door; production code outside
 * src/document imports from here (guard: boundary.guard.test.ts).
 * SURFACE.txt lists every export; new exports after batch 02b need a `# why:` justification.
 *
 * Reading. `node(id)`, `childIds`, `pageRoots`, `loadedPage`, `feedNames`,
 * `isLoaded` read the live Solid store on each call. Call them inside a tracking
 * scope (JSX, memo, effect) to re-render on change; a value read once in setup
 * is a snapshot, not a subscription. Returned values are readonly.
 * `loadRoutedPage` admits the first routed graph page and enables saves even
 * when no journal feed has loaded; O(blocks of that page), and an unsafe
 * replacement leaves the current edit intact. Satellite/scratch loads use
 * `ensurePageLoaded` and cannot enable persistence.
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
 * `persistentBlockRef` and `persistBlockRefTarget` wait for the target ID's
 * page save before the reference can enter persisted UI or edited content;
 * failure returns null or false. Cost is one target page save, plus a page
 * lookup when the target is not loaded. Callers need no debounce knowledge.
 *
 * Saving. Pages save as whole-page snapshots, never as operations. `markDirty`
 * requires an edit kind and schedules a trailing 400 ms save, capped at 3 s
 * from the first dirty mark of a burst;
 * the request carries distinct kinds in first-seen order. A lazy page also
 * declares `create-page` on its first save. Multi-page intents call
 * `persistTogether` with kinds inside the document module: their pages enter one open
 * group, overlapping groups merge, and the group sends one sinks-first
 * `savePages` request. A sealed request chains before later edits of its pages;
 * a new multi-page edit forms a successor group. Saves use the revision last
 * read, so an external file change surfaces a reasoned conflict rather than
 * an overwrite. `resolveConflict` reloads the pinned file for Use disk, or
 * guards Keep mine with the disk revision observed when the conflict arose;
 * a later disk edit raises a fresh conflict. For an alias-owner draft, Keep mine
 * appends the draft to the owner's current content at that observed revision.
 * `flushPage` / `flushAll` wait for
 * pending requests. An incomplete page-header draft remains dirty without an
 * autosave toast; exiting that editor reports invalid syntax once. A crash
 * between file writes can duplicate a moved block, but sinks-first order keeps
 * it on at least one file for acyclic moves (I-3). Creating a page file
 * goes through `createPage`, which refuses locally with a typed
 * `CreatePageRefusal`, distinct from a disk conflict. Only save/engine.ts calls
 * the backend's savePages/deletePage (I-1). Direct native page writers declare
 * their fixed kinds at their backend call and store entry. This adds O(1) kind
 * bookkeeping per edit and no disk bytes; missing kinds are refused before a
 * save request. Callers need no knowledge of the debounce or save group state.
 *
 * Outside changes. `applyGraphChange` handles one watcher event using
 * `reloadDisposition`: an own-save echo keeps content and undo; a clean page
 * reloads, unless it is being edited or moved (then the change is skipped); a
 * dirty, saving or conflicted page becomes a conflict, including when its file
 * was removed; a clean removed page that is not being edited leaves its route;
 * results that land after a graph switch are dropped (I-20). This module never imports the router: route
 * and feed effects go through handlers the app installs
 * (`installExternalChangeUiHandler`, `installAliasDraftRouteHandler`,
 * `installHistoryRouteContextAdapter`). An effective title change uses the
 * exact physical path to rekey navigation, loaded page ownership and its save
 * baseline in that order. The UI installs `installPageIdentityNavigation`;
 * rekeying refuses a name collision or an unsafe external reload. */
export { blockIsGridView, node, childIds, pageRoots, loadedPage, feedNames, isLoaded, formatForBlock, formatForPage, mainPages, pageByName } from "./model";
export type { ReadonlyFeedPage as FeedPage, ReadonlyNode as Node } from "./model";
export { conflictReason, conflicts, createPage, CreatePageRefusal, flushAll, flushPage, groupedPages, installAliasDraftRouteHandler, isConflicted, isDirty, isSaving, markDirty, refuseConflictedMove, resolveConflict, trackAssetWrite, waitingFor, waitingOn } from "./save/engine";
export { applyGraphChange, installExternalChangeUiHandler } from "./external";
export { appendFeed, deletePage, ensurePageLoaded, loadFeed, loadGuidePages, loadRoutedPage, registerPaneRouteProvider, reloadHlsIfLoaded, resetStore, restoreTodayJournalInFeed } from "./workingSet";
export { installRenameRefreshHandler, renamePageOnDisk } from "./graphRewrite";
export { emptyPage, resolveGuideBlockRef, resolveGuidePageDto, withToday, toLoadablePage, carryTodayPage, captureScratchPage, journalTemplatePage, demoJournalPage, switcherPage, queryWorkspacePage } from "./convert";
export { depthOf, nextVisible, pageVisibleOrder, prevVisible, visibleOrder } from "./tree";
export type { OutlineScope } from "./tree";
export { installHistoryRouteContextAdapter, redo, toggleUndoRedoMode, undo, withUndoUnit } from "./history";
export type { HistoryRouteContext } from "./history";
export { deleteBlock, ensureEmptyBlock, indentBlock, insertEmptyChildBlock, insertOutlineAfter, insertOutlineChildren, mergeWithPrev, outdentBlock, replaceChildOrders, replaceEmptyBlockWithOutline, revealNode, setCollapsed, setRaw, splitBlock, toggleCollapse } from "./edits/blocks";
export { pasteClipboardPayload, sanitizeOutlineIdsForPaste } from "./edits/paste";
export { appendToTodayJournal, captureToPage } from "./edits/capture";
export { beginPageHeaderEdit, blockPageReadOnly, blockProperty, blockWritable, collapsibleDescendantIds, finishPageHeaderEdit, makeOwnNumberedList, orderedListMarker, promotePagePreamble, readPageProperties, readPageProperty, readSchedule, removeOwnNumberedList, setBlockProperty, setCollapsedDeep, setCollapsedDescendants, setHeading, setPageProperty, setSchedule, stopOwnNumberedListOnEmptyEnter, toggleBlockProperty, toggleListItemAtIndex, toggleOwnNumberedList } from "./edits/properties";
export { blockExternalId, blockRef, ensureBlockId, persistBlockRefTarget, persistentBlockRef, resolveBlockRef } from "./edits/identity";
export { blockSubtreeMarkdown, buildClipboardPayload, dtoSubtreeMarkdown, exportNodesFor } from "./edits/serialize";
export { clearSelection, cycleSelectionTasks, deleteSelection, extendSelectionTo, hasSelection, indentSelection, isSelected, moveSelection, outdentSelection, selectBlock, selectedIds, selectionMarkdown } from "./edits/selection";
export { extendFeedForScroll, isBlockMoving, moveBlock, moveBlockFeed, moveItem, moveSelectionItems, nextVisibleOrExtend, setFeedExtender, withBlockMoving } from "./edits/moves";
export { installPageIdentityNavigation, rekeyPageIdentityByPath } from "./workingSet";
export { carryUnfinished } from "./edits/carry";

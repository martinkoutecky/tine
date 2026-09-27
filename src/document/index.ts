/** Document boundary: callers read live accessors and write through intents.
 * Cross-page removals wait for destination saves; watcher changes use
 * reloadDisposition before replacing a loaded page. */
export { blockIsGridView, node, childIds, pageRoots, loadedPage, feedNames, isLoaded, formatForBlock, formatForPage, mainPages, pageByName } from "./model";
export type { ReadonlyFeedPage as FeedPage, ReadonlyNode as Node } from "./model";
export { clearConflict, conflicts, createPage, flushAll, flushPage, forceSave, installAliasDraftRouteHandler, isConflicted, isDirty, isSaving, markConflict, markDirty, pageInstanceGeneration, trackAssetWrite } from "./save/engine";
export { appendFeed, deletePage, ensurePageLoaded, forgetPage, loadFeed, loadGuidePages, loadSingle, registerPaneRouteProvider, reloadDisposition, reloadHlsIfLoaded, reloadPage, resetStore, restoreTodayJournalInFeed } from "./workingSet";
export { emptyPage, pageToDto, resolveGuideBlockRef, resolveGuidePageDto, withToday, toLoadablePage, carryTodayPage, captureScratchPage, journalTemplatePage, demoJournalPage, switcherPage, queryWorkspacePage } from "./convert";
export { depthOf, nextVisible, pageVisibleOrder, prevVisible, trailingVisibleEmptyLeaf, visibleOrder } from "./tree";
export type { OutlineScope } from "./tree";
export { historyPageOnlyMode, installHistoryRouteContextAdapter, redo, toggleUndoRedoMode, undo, withUndoUnit } from "./history";
export type { HistoryRouteContext } from "./history";
export { deleteBlock, ensureEmptyBlock, indentBlock, insertEmptyChildBlock, insertOutlineAfter, insertOutlineChildren, mergeWithPrev, outdentBlock, replaceChildOrders, replaceEmptyBlockWithOutline, revealNode, setCollapsed, setRaw, splitBlock, toggleCollapse } from "./edits/blocks";
export { pasteClipboardPayload } from "./edits/paste";
export { appendToTodayJournal, captureToPage } from "./edits/capture";
export { beginPageHeaderEdit, blockPageReadOnly, blockProperty, blockWritable, collapsibleDescendantIds, finishPageHeaderEdit, makeOwnNumberedList, orderedListMarker, promotePagePreamble, readPageProperty, readSchedule, removeOwnNumberedList, setBlockProperty, setCollapsedDeep, setCollapsedDescendants, setHeading, setPageProperty, setSchedule, stopOwnNumberedListOnEmptyEnter, toggleBlockProperty, toggleListItemAtIndex, toggleOwnNumberedList } from "./edits/properties";
export { blockExternalId, blockRef, ensureBlockId, existingBlockId, persistBlockRefTarget, persistentBlockRef, rawWithBlockId, resolveBlockRef } from "./edits/identity";
export { blockSubtreeMarkdown, buildClipboardPayload, dtoSubtreeMarkdown, exportNodesFor } from "./edits/serialize";
export { clearSelection, cycleSelectionTasks, deleteSelection, extendSelectionTo, hasSelection, indentSelection, isSelected, moveSelection, outdentSelection, selectBlock, selectedIds, selectionMarkdown } from "./edits/selection";
export { extendFeedForScroll, isBlockMoving, moveBlock, moveBlockFeed, moveItem, moveSelectionItems, nextVisibleOrExtend, prepareCrossPageSources, setBlockMoving, setFeedExtender } from "./edits/moves";
export { carryUnfinished } from "./edits/carry";

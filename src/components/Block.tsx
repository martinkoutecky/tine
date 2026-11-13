import { shortcutKey } from "../shortcutKey";
import { plainTextPaste } from "../editor/plainTextPaste";
import { autoEmbedVideoLinks } from "../pasteSettings";
import { bindCollapseCommandSurface } from "../collapseCommandSurface";
import { msg, t as translate } from "../i18n";
import { codeWrapping } from "../codeDisplay";
import { stickyParents } from "../stickyParents";
import { StickyAncestor } from "./StickyAncestor";
import { AstBody } from "../render/body";
import { BlockRenderBoundary } from "./BlockRenderBoundary";
import { codeFences, caretInFence, caretOnOpeningFence, caretInDisplayMath } from "../editor/fences";
import { LineGutter } from "../render/LineGutter";
import { Show, Switch, Match, For, createMemo, createSignal, createContext, useContext, createUniqueId, createEffect, onMount, onCleanup, type JSX } from "solid-js";
import { editorCompletions, propertyNameInputCaret } from "../editor/editorCompletions";
import { reportUiFailure } from "../uiFailure";
import { clearClipboardPayload, normalizeClipboardText, peekClipboardPayload, readClipboardText } from "../clipboard";
import {
  autoPairEdit,
  applyFullWidthRefReplace,
  applyFullWidthCloserThrough,
} from "../editor/autocomplete";
import { pluginManager } from "../plugins/manager";
import { autoPairInsertOnInput, wrapSelectionEdit, doubleRefKind, backspacePairEdit, SELECTION_WRAP } from "../editor/autopair";
import { typoTypeReplace } from "../render/typography";
import { rangeInLiteral } from "../editor/inlineLiteral";
import { spellcheckEnabled } from "../spellcheckSettings";
import { restoreMovedSelection, keepCaretThroughRewrite, syncTextareaToStore } from "../editor/restoreMovedSelection";
import { navigateJournalDown } from "../editor/journalNavigation";
import { focusMountedEditor } from "../editor/surfaceFocus";
import { pageByName, blockPageReadOnly, blockWritable, setRaw, makeOwnNumberedList, removeOwnNumberedList, stopOwnNumberedListOnEmptyEnter, indentBlock, outdentBlock, mergeWithPrev, mergeWithNext, setCollapsed, beginPageHeaderEdit, finishPageHeaderEdit, exitMultilineBlock, insertOutlineAfter, insertSiblingAfterSubtree, insertEditorOutline, outlineFits, pasteClipboardPayload, sanitizeOutlineIdsForPaste, deleteBlock, moveBlockFeed, moveItem, selectBlock, selectBlockSubtree, moveSelection, isSelected, isBlockMoving, withBlockMoving, orderedListMarker, depthOf, blockExternalId, pinPageWhileDrafting, blockHasFoldableBody, blockHasPageEmbed, blockHasQueryBody, storedFold, type OutlineScope, node as docNode } from "../document";
import { pasteAsLinkEdit } from "../blockRefActions";
import { BlockBullet } from "./BlockBullet";
import {
  editingId,
  isEditingId,
  editingOwner,
  captureEditIntent,
  editingSurface,
  endEdit,
  noteSurfaceFocused,
  registerHistoryEditorTarget,
  startEditing,
  takeCaretFor,
  type CaretPos,
  takeHistoryEditorSelectionFor,
} from "../editorController";
import { OUTLINE_MAX_SOURCE_CHARS, pastedPlainBlocks, type OutlineNode } from "../editor/outline";
import { structuredHtmlPaste } from "../editor/htmlPaste";
import {
  toggleInlineFormat,
  insertLink,
  wrapLink,
  isPasteableUrl,
  videoPasteMacro,
  type Edit,
  type InlineFormat,
} from "../editor/format";
import { textCommandHandlers } from "../editor/textCommands";
import { captureKeyIntent } from "../editor/captureKeys";
import {
  essentialSelectionActions,
  secondarySelectionActions,
  type SelectionAction,
} from "../editor/selectionActions";
import { selectionActionLabel } from "./SelectionActionLabel";
import { effectiveHeadingLevel, facetsOf, headingPropertyFacets, EMPTY_FACETS, type Facets } from "../render/facets";
import { CopyButton } from "../render/inline";
import { touchPrimary } from "../inputCapability";
import { openJournalDatePicker } from "../journalSlash";
import { calcSource, serializeCalcExitCommit, evalCalc } from "../editor/calc";
import { insertYoutubeTimestamp } from "./youtubeTimestampEdit";
import { Rendered, detectMacro } from "./Rendered";
import { SeenRemoved, useSeenRow, useSeenThread } from "../seen/SeenHeader";
import { CollapseAllBorder, NO_THREAD_LINES, rowDecorationClasses, rowDecorationStyle, threadLineSettings, type ThreadLineDecoration } from "./block/rowChrome";
import { workflow, zoomInto, openContextMenu, openDatePicker, autoPairing, typographyMode, blockReferencesRequest, documentMode, docModeEnterForNewBlock } from "../ui";
import { ResultWriteTargetContext, liveWriteTarget } from "./queryResultIdentity";
import { toggleViewFold, viewFoldSurface } from "./block/readOnlyFold";
import { pushToast } from "../toasts";
import { copyBlockLink } from "./blockLinkCopy";
import { editorAssets } from "../editor/editorAssets";
import { clipboardEventFlavors } from "../editor/clipboardFlavors";
import { captureBinding, bindingCurrent } from "../binding";
import { graphOwner, readOwned } from "../owned";
import { EditorAutocomplete } from "./EditorAutocomplete";
import { parserReady } from "../render/parse";
import { BlockReferences } from "./BlockReferences";
import { editorCommandFor, isPermittedTabGesture, isTabLikeEvent } from "../keybindings";
import { cycleMarkerSmart } from "../editor/repeat";
import { registerTransientLayer } from "../transientLayers";
import {
  caretAtFirstRow,
  caretAtLastRow,
  caretColumnOnVisualRow,
  caretOffsetOnLastRow,
  textareaCaretLineRect,
} from "../editor/caretRows";
import { revealCaretColumn, revealCaretSettled } from "../editor/caretReveal";
import { selectionToolbarPlacement } from "./block/selectionToolbarPlacement";
import { splitProps, BUILTIN_HIDE, SHEET_CELL_HIDE, HIDE_ALL, isEditorEmpty, caretOnPropertyLine, isPropertiesOnly, multilineExitTrim } from "../editor/properties";
import { propertyEditorSession } from "../editor/propertySession";
import { blockNodeBinding } from "../editor/nodeBinding";
import { editorCommitRecovery } from "../editor/commitRecovery";
import { commitBlockEditorExit, editorMarkerBaseline } from "../editor/commitExit";
import { editorWindowFocus } from "../editor/windowFocus";
import { createCodeBodyEditor } from "../editor/codeBodyEditor";
import { codeBodyExitTrim, codeBodyProjection, codeFenceOnly } from "../editor/codeFence";
import { isAnnotationBlock, annotationInfo } from "../editor/annotation";
import { inPageFindPreservesEditorBlur } from "../inpageFind";
import { registerFocusedEditorCommandBridge, type MobileEditorCommandId } from "../editorCommandBridge";
import { blockListProps } from "./BlockList";
import { childrenSheetConfig } from "../sheet/childrenSheet";
import { SheetCellContext } from "../sheet/context";
import { appendSheetCellChild, structuralSheetPasteNode } from "../sheet/mutations";
import { cellBlockId, cellOwner, cellSurfaceKey, selectCellAfterEdit, moveCellAfterEdit, selectTopRowSeamAfterEdit } from "../sheet/selection";
import { forbidsEditEntry } from "../editor/editTargets";
import { SheetGrid } from "./SheetGrid";
import { SheetTable } from "./SheetTable";
import { SheetBoard } from "./SheetBoard";
import { SheetContainer } from "./SheetContainer";
import { ownsBlockContextMenu, shouldOpenBlockContextMenu } from "../contextMenuPolicy";
import { followBlockSwipe } from "./blockSwipeWiring";
import { bindRowHost, bindRowPosition, type RowHost } from "./block/embedRowHost";
import { beginEditGesture, dragId, dropInd, renderedClickOffset } from "./blockGestures";
import { bodyNeighbor, rawToWrite, type PendingEdit } from "../document";
import { noticeMoveOutOfView } from "../moveNotice";
import { captureEditorScrollAnchor } from "../editor/scrollAnchor";
import { PRIMARY_SELECTION_GUARD_PLACEHOLDER } from "../textareaPrimarySelection";
import { isBlockEmbedMacro, listLineAt, nearestScrollableY, resizeBlockEditor } from "./blockParts";
import { commentFromEditor } from "../commentActions";
import { setCollapsedBlocks } from "../document";
import { CommentMessages } from "./CommentMessages";
import { commentKey } from "../editor/commentKeys";
import { bodyScopeOf } from "../document";
import { quoteInGroup } from "./CommentGroup";
import { CommentBelowChip } from "./CommentBelowChip";
import { AuthorChip, CommentComposer, CommentHeader, CommentQuoteHeader, MarginMarker, QuoteHighlightProvider } from "./CommentParts";
import { commentEditorHide, commentFacts, marginNavigation, marginOutline } from "./blockComments";
import { BlockRefCountBadge } from "./BlockRefCountBadge";
import { AuthorGutter } from "./AuthorGutter";
import type { AuthorNeighbors } from "./authorRuns";
import { documentHasFocus, documentOf, isElementTag, listen, newResizeObserver, nextFrame, requestFrame, viewportOf } from "../windowRealm";
import { EditorGate } from "./EditorGate";
import { cm6EditorEnabled } from "../editor/cm6Setting";
import { noteLayoutDirAttr } from "../noteLayoutSetting";
import { arrowLeave, blockDirAttr } from "../render/textDirection";
import { editsLikeOutline, gridBlocksCommand, gridEditKey, gridNestedBlock, gridOutlineScope, openGridTarget, runGridOutline } from "../sheet/gridEditMove";
import { splitBlockInView } from "../editor/splitBlockInView";
import { sheetFaceGridId, sheetOutlineNeighbor } from "../sheet/outlineNavigation";
import { gridEntry } from "../sheet/gridNeighbour";
export { applySheetViewSlashAction } from "./block/sheetViewSlash";

// (The rendered-property hidden predicate is render/block.ts isRenderHiddenProp,
// shared with body.tsx's renderProps; its list is Rust's render_facets.rs.)

// Set ONLY by the quick-capture window (capture.tsx). Flows through the Block
// tree to every Editor so the capture's submit/cancel gestures and Enter mode
// work without prop-drilling. Absent (null) in the main app — normal editing.
export interface CaptureApi {
  submit: () => void;
  cancel: () => void;
  /** true → a plain Enter files; false → Enter is a new block, Cmd/Ctrl+Enter files. */
  enterFiles: () => boolean;
  /** Grey-italic placeholder for an empty capture bullet (e.g. "Edit as usual,
   *  Ctrl-Shift-Enter to submit"), with the live, configured submit shortcut. */
  bulletHint?: () => string;
  /** The capture WebView may only query page/tag candidates through its
   * dedicated native capability; it never receives the general graph route. */
  quickSwitch: (query: string, limit: number) => Promise<import("../types").PageEntry[]>;
}
export const CaptureCtx = createContext<CaptureApi | null>(null);

// Identifies the editing SURFACE a block is rendered in (the main pane vs a
// specific right-sidebar item). Defaults to "main"; RightSidebar overrides it per
// item. Used to arbitrate edit-focus when one block uuid renders in several
// surfaces at once (see startEditing's surface stamping).
export const SurfaceContext = createContext<string>("main");
export const OutlineScopeContext = createContext<OutlineScope | null>(null);
// GH #415: a block embed renders its target outline as a surface-local group. Up
// from the first visual row of the embed ROOT has no in-surface destination;
// LiveRefGroup carries the embed's host block here so the caret exits to the
// block preceding the embed on the host page (OG leaves the embed upward instead
// of trapping the caret).
// D-20261030-EMB: `rowHost` names the host a whole-block embed's one root row stands for.
export const EmbedNavExitContext = createContext<{ hostBlockId: string; firstRoot: () => string | undefined; rowHost: (id: string) => RowHost | null } | null>(null);
/** A plain bullet click opens the block here instead of zooming the pane (a
 *  hover preview opens it in a new tab, D-20261023-VQ). */
export const BulletOpenContext = createContext<((id: string) => void) | null>(null);
export interface CollapseSurfaceApi {
  collapsed: (id: string, stored: boolean) => boolean;
  toggle: (id: string, current: boolean) => void;
  setMany: (ids: readonly string[], collapsed: boolean) => void;
}
// Deliberate Tine divergence from OG Logseq: a block embed's root follows the
// source until its macro host records an explicit occurrence-owned collapse
// override; all inner page/block embed folds are the source's (D-20261030-EMB,
// GH #868). Reference/query results and read-only sources keep presentation
// folds. Keep the surface contract explicit when changing collapse parity.
export const CollapseSurfaceContext = createContext<CollapseSurfaceApi | null>(null);

/** Render and edit one document block through the document door. Work scales
 * with its visible descendants; a failed structured paste shows fixed text. */
export function Block(props: { id: string; hideRefCount?: boolean; forceExpanded?: boolean; authorNeighbors?: AuthorNeighbors }): JSX.Element {
  // Share one node read across this block's derivations.
  const node = createMemo(blockNodeBinding(() => props.id).node);
  const propertySession = propertyEditorSession();
  // Unique per rendered instance, so when one block uuid appears in several
  // surfaces only the instance that was clicked mounts the editor (the rest stay
  // rendered and reflect edits live). null owner = unscoped (keyboard nav).
  const instanceId = createUniqueId();
  // This block's edit "surface": the main pane, a sidebar item, or a secondary
  // "ref:…" reference view (agenda / {{query}} / {{embed}} / linked+block refs —
  // all keyed by LiveRefGroup). Drives which instance shows the editor.
  const surfaceKey = useContext(SurfaceContext);
  const outlineScope = useContext(OutlineScopeContext);
  const collapseSurface = useContext(CollapseSurfaceContext);
  const viewCollapseSurface = () => viewFoldSurface(props.id, collapseSurface);
  const bulletOpen = useContext(BulletOpenContext);
  const seenRow = useSeenRow(props.id);
  const seenThread = useSeenThread();
  const editing = () => {
    if (!isEditingId(props.id)) return false;
    const owner = editingOwner();
    // Scoped (a click): only the exact instance that was clicked edits; every other
    // instance of this uuid stays rendered and reflects the edit live.
    if (owner !== null) return owner === instanceId;
    const scopedSurface = editingSurface();
    if (scopedSurface !== null) return scopedSurface === surfaceKey;
    // Unscoped (keyboard nav / split): edit in the PRIMARY surface where the caret
    // already was. A block that also appears in a secondary "ref:" surface (e.g. the
    // journal agenda re-lists today's scheduled/deadline bullets) must stay RENDERED
    // there — arrowing into the real bullet must not flip the agenda copy into an
    // editor. (Clicking a ref/agenda copy still edits it in place, via the branch
    // above.) Matches the sidebar rule: edit where you're editing, render elsewhere.
    return !surfaceKey.startsWith("ref:") && !surfaceKey.startsWith("embed:");
  };
  const storedFolded = () => storedFold(props.id); // `collapsed::`, else a query's authored `:collapsed? true` (GH #619 Q8)
  const embedRow = useContext(EmbedNavExitContext);
  const rowHost = () => embedRow?.rowHost(props.id) ?? null;
  bindCollapseCommandSurface(props.id, collapseSurface, outlineScope, editing, rowHost);
  const collapsed = () => viewCollapseSurface()?.collapsed(props.id, storedFolded()) ?? storedFolded();
  const fmt = createMemo(() => pageByName(node().page)?.format ?? "md");
  const blockFacets = createMemo<Facets>(() => {
    const n = node();
    return n ? propertySession.facets(n.raw, fmt()) : EMPTY_FACETS;
  });
  const { outlineChildren, marginComments, commentChildren, quoteCoverage } = marginOutline(node, surfaceKey, fmt);
  const groupedQuote = quoteInGroup(props.id);
  const hasChildren = () => outlineChildren().length > 0;
  const hasFoldableBody = () => hasChildren() || blockHasPageEmbed(props.id) || blockHasQueryBody(props.id); // a query host folds its results (OG; GH #619)
  // The children-source sheet this block owns (a query block's table/board is the macro's).
  const sheet = createMemo(() => childrenSheetConfig(blockFacets().properties, node().raw));
  // Only ordinary-outline parents (including folded ones) read plugin decoration.
  const threadLineDecoration = (): ThreadLineDecoration => {
    if (!(hasChildren() && sheet().view === null)) return NO_THREAD_LINES;
    return threadLineSettings(pluginManager.hasDeclarativeDecoration("thread-lines"),
      key => pluginManager.declarativeDecorationSetting("thread-lines", key));
  };
  const headingLevel = createMemo(() => effectiveHeadingLevel(blockFacets(), depthOf(props.id)));
  // Editor-only: page load never runs splitProps over every block's text.
  const editorVisibleValue = () => {
    const n = node();
    if (!n) return "";
    const format = pageByName(n.page)?.format === "org" ? "org" : "md";
    return splitProps(n.raw, BUILTIN_HIDE, format).visible;
  };
  const editorIsUniline = () => !editorVisibleValue().includes("\n");
  // Block-level "linked references" panel toggled by the reference-count badge.
  const [showRefs, setShowRefs] = createSignal(false);
  createEffect(() => {
    const requested = blockReferencesRequest()?.id;
    if (requested && parserReady()
      && (requested === props.id || requested === blockExternalId(props.id))) setShowRefs(true);
  });
  // Ordered-list label for THIS block's own bullet (OG numbers the block itself,
  // not its children); null for a normal bullet.
  const orderMarker = () => orderedListMarker(props.id, blockFacets().properties);
  // An org page Tine can't round-trip is shown but NOT editable (Tine must never
  // rewrite it). Clicking a block doesn't enter the editor on such a page.
  const readOnly = () => blockPageReadOnly(props.id);
  // D-20261030-EMB (0.8): a whole-block `{{embed ((uuid))}}` host shows no
  // controls of its own; the source root's row is its one bullet, standing at
  // the host's position (`rowHost`). Page embeds and hosts with text keep theirs.
  const macro = createMemo(() => detectMacro(node().raw, fmt())); // shared with `Rendered`
  const blockEmbedHost = createMemo(() => isBlockEmbedMacro(macro()));
  const { commentSelector, storedAuthor, author, showsAuthor, bulletless, commentClasses } = commentFacts(node, () => blockFacets().properties, fmt, { id: () => props.id, neighbours: () => props.authorNeighbors });
  const hasCommentIdentity = createMemo(() => author() !== null || bulletless());
  let mainEl: HTMLDivElement | undefined; const expandThread = () => { const surface = viewCollapseSurface(); if (surface) surface.setMany([props.id], false); else setCollapsedBlocks([props.id], false); };
  const commentPresentation = createMemo(() => hasCommentIdentity() ? {
    quote: <CommentQuoteHeader parentId={node().parent} selector={groupedQuote() || !bulletless() ? null : commentSelector()} />,
    header: <Show when={bulletless()}><CommentHeader id={props.id} author={author()} folded={collapsed()} toggleFold={() => toggleViewFold(props.id, viewCollapseSurface(), collapsed())} /></Show>,
    gutter: <AuthorGutter author={bulletless() || blockEmbedHost() ? null : author()} />,
    composer: <Show when={commentClasses()["comment-card"]}><CommentMessages id={props.id} collapsed={collapsed()} expand={expandThread} /><CommentComposer id={props.id} surface={surfaceKey} card={() => mainEl?.parentElement} collapsed={collapsed} expand={expandThread} /></Show>,
  } : null);
  const stickyParent = () => stickyParents() && !bulletless() && (props.forceExpanded || !collapsed()) && (macro() ? blockHasPageEmbed(props.id) && !editing() : hasChildren());

  return (
    <div
      class="ls-block"
      dir={noteLayoutDirAttr()}
      {...(() => {
        const thread = threadLineDecoration();
        return {
          style: rowDecorationStyle(thread, thread.enabled && thread.levels ? depthOf(props.id) : 0),
          classList: {
            collapsed: collapsed(),
            "block-embed-host": blockEmbedHost(),
            ...commentClasses(),
            // The card of a thread with a message changed since the user last looked (DESIGN §12),
            // also while the thread is folded to "N replies" and its message rows are not drawn.
            "comment-seen-changed": !!commentClasses()["comment-card"] && seenThread(props.id),
            ...rowDecorationClasses(thread),
          },
        };
      })()}
      data-block-id={props.id} data-author={storedAuthor() ?? undefined}
      ref={(el) => { if (embedRow) bindRowHost(el, rowHost); bindRowPosition(el, () => rowHost() ?? { id: props.id, scope: outlineScope }); }}
      data-sticky-parent={stickyParent() ? "" : undefined}
      data-block-ref={parserReady() ? blockExternalId(props.id, propertySession.identity(node().raw, fmt())) ?? props.id : undefined}
    >
      <Show when={stickyParent() ? macro()?.kind ?? "outline" : false} keyed><StickyAncestor id={props.id} raw={macro()?.kind === "embed" ? macro()!.inner.slice("embed ".length) : node().raw} /></Show>
      {commentPresentation()?.quote}
      {commentPresentation()?.header}
      <div
        class="block-main"
        ref={(el) => {
          mainEl = el;
          // GH #501: touch swipes on the row while a touch input exists, following
          // it if it appears or goes away (src/blockSwipe.ts).
          followBlockSwipe(el, { get id() { return rowHost()?.id ?? props.id; }, get scope() { return rowHost() ? rowHost()!.scope : outlineScope; },
            editorId: props.id, editing, readOnly: () => blockPageReadOnly(rowHost()?.id ?? props.id) });
        }}
        classList={{
          // Heading level on the row so the bullet column can match the (taller)
          // heading line box and the bullet stays centered on the first line. While
          // editing, apply the same offset only when the hidden-props-stripped editor
          // value is still a single line; multi-line heading blocks edit at body size in the textarea. The CM6
          // editor sizes the first line whatever follows it.
          [`bullet-h${headingLevel()}`]: headingLevel() != null && (!editing() || cm6EditorEnabled() || editorIsUniline()),
          "drop-before": dropInd()?.id === props.id && dropInd()?.position === "before",
          "drop-after": dropInd()?.id === props.id && dropInd()?.position === "after",
          "drop-child": dropInd()?.id === props.id && dropInd()?.position === "child",
          dragging: dragId() === props.id,
          selected: isSelected(props.id),
          // Marks the row being edited; drives dim-mode's active-block spotlight.
          editing: editing(),
          "seen-changed": seenRow.changed(),
        }}
        onContextMenu={(e) => {
          // GH #726: a nested result/embed row owns its menu (contextMenuPolicy).
          if (!ownsBlockContextMenu(e.target, mainEl) || !shouldOpenBlockContextMenu(e.target)) return;
          e.preventDefault();
          openContextMenu(e.clientX, e.clientY, rowHost()?.id ?? props.id);
        }}
      >
        <Show when={!blockEmbedHost() && !bulletless()}>
        <div class="block-controls" data-sticky-row={macro()?.kind === "embed" ? "" : undefined}>
          <span
            class="collapse-toggle"
            classList={{ "has-children": hasFoldableBody() }}
            onClick={() => toggleViewFold(props.id, viewCollapseSurface(), collapsed())}
          >
            <Show when={hasFoldableBody()}>
              <svg viewBox="0 0 24 24" class="triangle">
                <path d="M8 5l8 7-8 7z" />
              </svg>
            </Show>
          </span>
          <BlockBullet id={props.id} owner={rowHost()?.id ?? props.id} closed={collapsed() && hasFoldableBody()}
            order={orderMarker()} open={() => bulletOpen ? bulletOpen(props.id) : zoomInto(props.id)} />
        </div>
        </Show>

        <div
          class="block-content-wrapper"
          classList={{ "read-only": readOnly() }}
          onMouseDown={(e) => {
            // Mousedown anywhere in the row (not on a link/chip) arms the same
            // click-or-drag gesture as block content, with an end-of-block caret
            // (the row padding has no text to map). Read-only org pages don't edit.
            if (e.button !== 0 || e.ctrlKey || e.metaKey || e.altKey) return;
            if (!editing() && !readOnly() && !forbidsEditEntry(e))
              beginEditGesture(e, props.id, docNode(props.id).raw.length, instanceId, outlineScope);
          }}
        >
          {commentPresentation()?.gutter}
          <Show
            when={editing() ? node() : undefined} keyed
            fallback={
              <QuoteHighlightProvider parentId={props.id} format={fmt()} when={node().children.length > 0} comments={commentChildren} coverage={quoteCoverage()}>
                {() => <Rendered
                  id={props.id}
                  node={node}
                  fmt={fmt}
                  facets={blockFacets}
                  headingLevel={headingLevel}
                  macro={macro}
                  owner={instanceId}
                  outlineScope={outlineScope}
                  refCountBadge={<>
                    <Show when={showsAuthor() && !bulletless()}>
                      <AuthorChip author={author()} menuOf={props.id} />
                    </Show>
                    <BlockRefCountBadge id={props.id} hidden={props.hideRefCount} open={showRefs()} toggle={() => setShowRefs((v) => !v)} />
                  </>}
                />}
              </QuoteHighlightProvider>
            }
          >
            <div class="authored-editor"><Show when={showsAuthor() && !bulletless()}><AuthorChip author={author()} menuOf={props.id} /></Show><Editor id={props.id} propertySession={propertySession} /></div>
          </Show>
        </div>
        <Show when={node().children.length > 0}><MarginMarker parentId={props.id} comments={marginComments()} source={commentChildren} coverage={quoteCoverage()} />
        <CommentBelowChip id={props.id} collapsed={collapsed() && !props.forceExpanded && !bulletless()} expand={() => toggleViewFold(props.id, viewCollapseSurface(), true)} /></Show>
      </div>
      <SeenRemoved first={props.id} />
      <Show when={showRefs()}>
        <div class="block-references">
          <BlockReferences id={props.id} />
        </div>
      </Show>
      <Show when={(props.forceExpanded || !collapsed()) && (hasChildren() || sheet().view === "grid" || sheet().view === "table" || sheet().view === "board")}>
        <Switch>
          <Match when={sheet().view === "grid"}>
            <SheetContainer>
              <SheetGrid id={props.id} />
            </SheetContainer>
          </Match>
          <Match when={sheet().view === "table"}>
            <SheetContainer>
              <SheetTable ownerId={props.id} rowSource="children" />
            </SheetContainer>
          </Match>
          <Match when={sheet().view === "board"}>
            <SheetContainer>
              <SheetBoard ownerId={props.id} rowSource="children" groupBy={sheet().groupBy} />
            </SheetContainer>
          </Match>
          <Match when={true}>
            <div class="block-children-container">
              <CollapseAllBorder id={props.id} readOnly={readOnly()} surface={viewCollapseSurface()} />
              <div class="block-children">
                <For {...blockListProps(outlineChildren, (id, neighbours) => <Block id={id} authorNeighbors={neighbours} />)} />
              </div>
            </div>
          </Match>
        </Switch>
      </Show>
      {commentPresentation()?.composer}
      <SeenRemoved after={props.id} />
    </div>
  );
}

const SHEET_CELL_BLOCKED_EDITOR_COMMANDS = new Set([
  // A cell is not an outline parent; the grid also owns mod+r (fill right).
  "editor/comment",
  "editor/indent",
  "editor/outdent",
  "editor/move-block-up",
  "editor/move-block-down",
  "editor/select-block-up",
  "editor/select-block-down",
  // The grid owns mod+a (whole-grid selection) — a cell editor must not
  // escalate into outline block selection.
  "editor/select-all",
  // A cell is not an outline sibling; Alt+Enter there appends a child (sheet keys).
  "editor/new-sibling-after-subtree",
]);


// Block editor for `props.id`. Inside quick capture, CaptureCtx repurposes
// Enter/Escape when autocomplete is closed to commit or dismiss the capture.

export function Editor(props: { id: string; propertySession?: ReturnType<typeof propertyEditorSession> }): JSX.Element {
  return <EditorGate {...props} surface={useContext(SurfaceContext)} scope={useContext(OutlineScopeContext)}
    classic={() => <EditorEngine {...props} />} />;
}

function EditorEngine(props: { id: string; propertySession?: ReturnType<typeof propertyEditorSession> }): JSX.Element {
  const propertySession = props.propertySession ?? propertyEditorSession();
  propertySession.begin();
  // Non-null only inside the quick-capture window (see CaptureCtx).
  const cap = useContext(CaptureCtx);
  const sheetCell = useContext(SheetCellContext);
  // Which surface (main pane / a specific sidebar item) this editor lives in —
  // drives edit-focus arbitration when the same block renders in several surfaces.
  const surfaceKey = useContext(SurfaceContext);
  const inheritedScope = useContext(OutlineScopeContext);
  const outlineScope = sheetCell && editsLikeOutline(sheetCell) ? gridOutlineScope(sheetCell) : inheritedScope;
  const embedNavExit = useContext(EmbedNavExitContext);
  // Inside query results, the editor's pickers write only if no result changed meanwhile (GH #433 review).
  const inResults = useContext(ResultWriteTargetContext);
  const resultTarget = (id: string) => inResults(id) ? liveWriteTarget(id) : undefined;
  const collapseSurface = useContext(CollapseSurfaceContext);
  // Ref/query navigation and Enter's child stay in the rendered occurrence.
  // Other structural destinations may leave the results and use the source
  // outline (GH #341). Embeds keep both in their transclusion.
  const navigationSurface = () =>
    surfaceKey.startsWith("ref:") || surfaceKey.startsWith("embed:") ? surfaceKey : null;
  const editSurface = () => surfaceKey.startsWith("embed:") ? surfaceKey : null;
  // A navOnly display-list scope must never act as a merge/structural topology.
  const structuralScope = outlineScope?.navOnly ? null : outlineScope;
  // Where this row sits: a whole-block embed's root row stands for its host (D-20261030-EMB).
  const position = (): RowHost => embedNavExit?.rowHost(props.id) ?? { id: props.id, scope: outlineScope };
  const setOccurrenceCollapsed = (value: boolean) => {
    if (!collapseSurface) { setCollapsed(props.id, value); return; }
    const n = docNode(props.id);
    if (!n || !blockHasFoldableBody(props.id)) return;
    const current = collapseSurface.collapsed(props.id, n.collapsed);
    if (current !== value) collapseSurface.toggle(props.id, current);
  };
  const { viewPrev, viewNext } = marginNavigation(props.id, outlineScope, surfaceKey, () => pageFmt());
  let ref!: HTMLTextAreaElement;
  let pendingScrollAnchor: ReturnType<typeof captureEditorScrollAnchor> | undefined;
  onCleanup(() => pendingScrollAnchor?.cancel());
  let editorMounted = true;
  onCleanup(() => {
    editorMounted = false;
  });
  const autocompleteLayerId = `block-completion-${createUniqueId()}`;
  const selectionOverflowLayerId = `block-selection-overflow-${createUniqueId()}`;
  const { node, shownNode, live: liveNode, current: editorNodeCurrent, writable: editorNodeWritable } = blockNodeBinding(() => props.id, () => editorMounted);
  const replacedUnderEditor = () => !editorNodeWritable();
  const sheetInitialRaw = sheetCell ? node()?.raw ?? "" : null;
  // Page format drives in-block list markers (`-` is an org bullet, not md).
  const pageFmt = (): "md" | "org" => (pageByName(node().page)?.format === "org" ? "org" : "md");
  const isFirstPagePropertiesBlock = (raw: string) => {
    const page = pageByName(node().page);
    const propertyDraft = isPropertiesOnly(raw)
      || (raw.endsWith("\n") && isPropertiesOnly(raw.slice(0, -1)));
    return page?.format === "md"
      && page.roots[0] === props.id
      && (node().originatedFromPageHeader || (!page.preBlock && propertyDraft));
  };
  const commentHide = commentEditorHide(node(), propertySession.facets(node().raw, pageFmt()).properties, pageFmt(), docNode);
  const isAnnot = () => annotationInfo(propertySession.facets(node().raw, pageFmt()).properties) !== null;
  const hideFn = () => (isAnnot() ? HIDE_ALL : sheetCell ? SHEET_CELL_HIDE : commentHide);
  const editorParts = createMemo(() => propertySession.split(node().raw, hideFn(), pageFmt()));
  const editorValue = () => editorParts().visible;
  // GH #357: while the buffer IS one whole-block code fence the editor presents
  // as the same mono, no-wrap card the rendered face is (no re-layout jump).
  // Mixed content / ```calc keep their own modes; re-derived per keystroke.
  // OG parity: `components/editor.cljs get-editor-style-class` takes the level from the block's own `heading::`
  // property (`(:heading (:block/properties block))`), else from the content's `#` prefix, and gates on the content
  // being one line (`uniline-block`). The typed text answers the `#` prefix live; the hidden `heading::` property
  // is not in the editor text, so it comes from the whole block.
  const editorHeadingLevel = createMemo(() => {
    const visible = editorValue();
    if (visible.includes("\n")) return null;
    const typed = effectiveHeadingLevel(facetsOf(visible, pageFmt()), depthOf(props.id));
    if (typed !== null) return typed;
    return effectiveHeadingLevel(headingPropertyFacets(propertySession.facets(node().raw, pageFmt()).properties), depthOf(props.id));
  });
  // Live calc preview: when this editor opened on a ```calc fence, show the SAME
  // results panel as the rendered view, recomputed on every keystroke (onInput
  // commits to node().raw live, so editorValue() is current). Matches OG's
  // calculator, which stays live while you type instead of only computing after
  // you exit.
  // A ```calc block edits like OG: the textarea shows ONLY the fence-stripped
  // expressions (calcLive), with a line-number gutter + live results beside it,
  // and the fence is re-added on commit. Calc mode is captured at editor mount,
  // not re-derived from the latest committed raw, so an exit commit can still
  // preserve the fence even if the committed raw is temporarily malformed.
  const [editingCalc, setEditingCalc] = createSignal(calcSource(editorValue()) !== null);
  const calcLive = createMemo(() => {
    if (!editingCalc()) return null;
    return calcSource(editorValue()) ?? editorValue();
  });
  const isCalc = editingCalc;
  const calcRows = createMemo(() => (isCalc() ? evalCalc(calcLive() ?? "") : []));
  let rawBeforeLastCommit = node().raw;
  const markerBaseline = editorMarkerBaseline(node().raw, pageFmt());
  const commitRecovery = editorCommitRecovery(() => ref, editorNodeCurrent, () => ref.value);
  /** The write `commit(text)` would make, decided but not made: `raw` is the block's text once written (the
   *  clock transition settled, so it runs once), `write` writes it. A refused join throws here, as in
   *  `commit`. A structural gesture that edits the text it acts on (the multiline exit's trim) asks the
   *  kernel about `raw` and writes only on an answer. */
  const stage = (text: string, opts?: { timetracking?: boolean; calc?: boolean }): PendingEdit => {
    const unchanged = (): PendingEdit => ({ raw: node().raw, write() {} });
    // A disposed editor whose block was replaced never writes (see shownNode).
    if (replacedUnderEditor()) return unchanged();
    const commitAsCalc = opts?.calc ?? isCalc();
    // For calc, `text` is the bare expressions the user sees — re-fence it.
    // Keep any trailing space the user left (or that a `/priority` insert added as
    // a typing convenience) in the live buffer — OG keeps it while you edit and
    // only trims when the block is written to disk. We do the SAME: the trailing
    // trim now lives at the save boundary (toDto in store.ts), not here. Trimming
    // here re-synced the reactive textarea (`value={editorValue()}`) to the
    // trimmed text on every keystroke, so backspacing to a trailing space ate the
    // space out from under the caret — the block-eats-the-space bug.
    // No-op commit (focus/blur with no real edit): if the editor-visible text is
    // unchanged, don't rewrite. Needed for org, where reattaching the hidden
    // drawer canonicalizes its position — so `next === raw` alone wouldn't catch
    // a block whose drawer wasn't already canonical, and would churn the file.
    if (!commitAsCalc && !codeShown() && text === editorValue()) { commitRecovery.applied(); return unchanged(); }
    // For a code wrapper `text` is the payload body: re-attach the exact wrapper
    // bytes (GH #412/#413: the body-only projection is reversible).
    const visible = commitAsCalc ? serializeCalcExitCommit(text, editorValue()) : text;
    const next = commitRecovery.join(() => (!commitAsCalc ? codeWrapCommit(text) : null) ?? propertySession.join(visible, editorParts().hidden, node().raw, hideFn(), pageFmt()), text);
    if (next === node().raw) return unchanged();
    const setRawOpts = opts && "timetracking" in opts ? { timetracking: opts.timetracking } : undefined;
    const settled = rawToWrite(props.id, next, { ...setRawOpts, markerBaseline: markerBaseline(node().raw) });
    return {
      raw: settled,
      write() {
        // GH #515: capture once for the autosize frame, before live mirrors above react.
        if (pendingScrollAnchor === undefined) {
          pendingScrollAnchor = ref && documentOf(ref).activeElement === ref
            ? captureEditorScrollAnchor(ref, nearestScrollableY(ref)) : null;
        }
        autosize();
        rawBeforeLastCommit = node().raw;
        keepCaretThroughRewrite(ref, !commitAsCalc && !codeShown() && !compositionActive, () => setRaw(props.id, settled, { timetracking: false }));
      },
    };
  };
  const commit = (text: string, opts?: { timetracking?: boolean; calc?: boolean }) => stage(text, opts).write();
  /** Taking `from..to` out of the editor and the block, decided but not made: the textarea's counterpart of
   *  the CodeMirror editor's `deleteSelection`. Enter over a selection and a template completion's trigger
   *  removal stage it, ask the kernel about the text it leaves and write it only once it has answered. */
  const deleteRange = (from: number, to: number): PendingEdit => {
    const text = ref.value.slice(0, from) + ref.value.slice(to);
    const staged = stage(text);
    return {
      raw: staged.raw,
      write() {
        staged.write();
        queueMicrotask(() => {
          if (!editorMounted) return;
          syncTextareaToStore(ref, text, textareaValue(), from);
          autosize();
        });
      },
    };
  };

  // Nest/un-nest an in-block list item by ±2 leading spaces (Tab/Shift-Tab when
  // the caret is on a `+`/`*`/ordered list line).
  // In-block list line at the caret. A body-only code view (and a calc block) is all literal text, so it
  // has no list lines; a raw view asks the parser which lines are literal (blockParts `listLineAt`).
  const listLine = (text: string, caret: number) =>
    codeShown() !== null || isCalc() ? null : listLineAt(text, caret, pageFmt());
  const nudgeListItem = (ll: NonNullable<ReturnType<typeof listLineAt>>, delta: number) => {
    const text = ref.value;
    const caret = ref.selectionStart;
    if (delta > 0) {
      const c = caret + 2;
      applyEdit({ text: text.slice(0, ll.lineStart) + "  " + text.slice(ll.lineStart), start: c, end: c });
    } else {
      const lead = Math.min(2, ll.indent.length);
      const c = Math.max(ll.lineStart, caret - lead);
      applyEdit({ text: text.slice(0, ll.lineStart) + text.slice(ll.lineStart + lead), start: c, end: c });
    }
  };

  const completion = editorCompletions({
    id: props.id, format: pageFmt,
    current: () => editorMounted && !!liveNode() && !!ref?.isConnected,
    composing: () => compositionActive,
    // CM6-D9: a projected code body is code, as in CM6 (detectTrigger owns the raw-fence rule).
    suppressTriggers: () => isCalc() || isAnnot() || !!sheetCell || codeShown() !== null,
    text: () => ref.value, caret: () => ref.selectionStart,
    selection: () => [ref.selectionStart, ref.selectionEnd],
    rawBeforeCommit: () => rawBeforeLastCommit, visible: editorValue,
    stageDelete: deleteRange,
    apply: (raw, start, end = start, focus = true, restoreGuard) => {
      commit(raw);
      queueMicrotask(() => {
        if (!editorMounted || (restoreGuard && !restoreGuard())) return;
        syncTextareaToStore(ref, raw, textareaValue(), start, end);
        if (focus) ref.focus();
        autosize();
        updateAcRect();
      });
    },
    replace: (raw, caret, close) => replaceCompletion(raw, caret, close),
    // DOM selection settles after Solid mirrors the committed text.
    afterEdit: callback => queueMicrotask(() => { if (editorMounted) { callback(); updateAcRect(); } }),
    languagePicker: active => { scaffoldLanguagePicker = active; },
    closed: () => {
      const scaffold = scaffoldLanguagePicker ? codeBodyProjection(editorValue(), pageFmt()) : null;
      scaffoldLanguagePicker = false;
      if (scaffold) {
        codeView.enter(scaffold.open.length);
        queueMicrotask(() => {
          ref.value = scaffold.body;
          ref.setSelectionRange(0, 0);
          autosize();
        });
      }
    },
    element: () => ref, resultTarget: () => resultTarget(props.id),
    openJournal: () => queueMicrotask(() => {
      if (editorMounted) openJournalDatePicker(ref, commit, () => editorMounted, autosize);
    }),
    assets: {
      uploadAsset: () => assets.uploadAsset(),
      desktopVoiceMemoToggle: () => assets.desktopVoiceMemoToggle(),
      createDrawioDiagram: () => assets.createDrawioDiagram(),
    },
    quickSwitch: cap ? (query, limit) => cap.quickSwitch(query, limit) : undefined,
    refOwner: () => { const binding = captureBinding(); return () => bindingCurrent(binding); },
  });
  const { ac, acItems, acBlockState, acIndex, setAcIndex, acVisible, closeAc,
    selectAc, updateAutocomplete, refresh: refreshAutocompleteAfterInput } = completion;
  const codeView = createCodeBodyEditor(editorValue, pageFmt, () => !sheetCell && !isCalc() && ac()?.kind !== "code-language", () => node().raw);
  const codeShown = codeView.shown;
  const mixedCodeContext = createMemo(() => {
    const p = codeShown();
    if (!p || !codeView.mixed()) return null;
    const raw = editorValue();
    const fence = codeFences(raw, pageFmt()).find(f => f.openEnd === p.open.length);
    return fence ? { before: raw.slice(0, fence.start), after: raw.slice(fence.end) } : null;
  });
  const codeEditing = createMemo(() => codeShown() !== null || codeFenceOnly(editorValue(), pageFmt()) !== null);
  const codeWrapCommit = codeView.join;
  // One door to the fence language picker: `/Code block` and the hand-typed ```
  // scaffold both come here (GH #507). While open `codeShown` keeps the raw
  // view so the opener line stays visible; choosing a language, or Escape,
  // then drops into the body-only view.
  let scaffoldLanguagePicker = false;
  const openFenceLanguagePicker = (raw: string, fenceEnd: number) =>
    completion.openLanguagePicker(raw, fenceEnd);
  let acListRef: HTMLDivElement | undefined;
  // The autocomplete popup is rendered through a Portal (fixed-positioned), so a
  // clipping ancestor — the right sidebar's `overflow:auto`, a modal — can't cut
  // it off. We anchor it to the textarea's viewport rect and recompute while it's
  // open (the editor grows as you type) and on scroll/resize.
  const [acRect, setAcRect] = createSignal<{ left: number; top: number; bottom: number } | null>(null);
  // The popup hangs against the CARET'S line (OG's caret-aware placement), not the
  // bottom of a tall editor: a page-properties block with many rows would
  // otherwise open its list below the last row, often off screen (GH #737). With
  // no layout (jsdom) it falls back to the editor's own box.
  const updateAcRect = () => {
    if (ref) {
      const r = ref.getBoundingClientRect();
      const line = textareaCaretLineRect(ref, ref.selectionStart);
      setAcRect({ left: r.left, top: line?.top ?? r.top, bottom: line?.bottom ?? r.bottom });
    }
  };
  createEffect(() => {
    ac(); // each trigger change (every keystroke) moves the caret line
    if (acVisible()) updateAcRect(); // re-anchor on open / each keystroke
  });
  // Flip the popup above the line when there isn't room below (near the viewport
  // bottom), and keep it inside the viewport's right edge, so it stays fully
  // visible - matches OG's caret-aware placement.
  const acStyle = (): Record<string, string> => {
    const r = acRect();
    if (!r) return {};
    const viewport = viewportOf(ref);
    const below = viewport.height - r.bottom;
    const openUp = below < 300 && r.top > below;
    // Popup width is content-sized (min 240px); measure the mounted one.
    const width = acListRef?.offsetWidth || 240;
    const left = Math.max(8, Math.min(r.left, viewport.width - width - 8));
    return openUp
      ? { left: `${left}px`, bottom: `${viewport.height - r.top + 2}px` }
      : { left: `${left}px`, top: `${r.bottom + 2}px` };
  };
  onMount(() => {
    const reanchor = () => { if (ac()) updateAcRect(); };
    // capture phase so an inner scroller (the feed, the sidebar body) also fires.
    // The editor's own window: a popup scrolls and resizes independently of main.
    onCleanup(listen(ref, "scroll", reanchor, true));
    onCleanup(listen(ref, "resize", reanchor));
  });

  // Page/block/tag/command/code completion is a real transient above its editor
  // and, on mobile, above the drawer. One Escape peels only this popup.
  createEffect(() => {
    if (!acVisible()) return;
    const unregister = registerTransientLayer({
      id: autocompleteLayerId,
      root: () => acListRef ?? null,
      trigger: () => ref ?? null,
      dismiss: () => { closeAc(); ref?.focus(); return true; },
    });
    onCleanup(unregister);
  });

  // Apply a pure text edit (format toggle / kill motion) to the textarea and
  // restore the resulting selection.
  const applyEdit = (ed: Edit) => {
    commit(ed.text);
    queueMicrotask(() => {
      syncTextareaToStore(ref, ed.text, textareaValue(), ed.start, ed.end, ed.direction);
      ref.focus();
      autosize();
    });
  };
  const moveCaret = (pos: number) => {
    ref.setSelectionRange(pos, pos);
  };

  // Floating selection toolbar (bold/italic/highlight/link) — shown while a
  // non-empty selection exists in this block's editor.
  const [hasSel, setHasSel] = createSignal(false);
  const [selectionOverflowOpen, setSelectionOverflowOpen] = createSignal(false);
  let selectionOverflowRef: HTMLDivElement | undefined;
  const selToolbar = selectionToolbarPlacement(() => ref, () => !touchPrimary() && hasSel());
  const updateSel = () => {
    if (!editorMounted || !ref.isConnected || !liveNode()) return;
    codeView.syncSelection(ref);
    const selected = ref.selectionStart !== ref.selectionEnd;
    setHasSel(selected);
    if (!selected) setSelectionOverflowOpen(false);
    closeAutocompleteIfOutside();
    queueMicrotask(selToolbar.position);
  };
  // OG `close-autocomplete-if-outside` (editor.cljs, on click and keyup): page,
  // tag and block search end once the caret leaves their reference; a stale
  // popup would keep Arrow/Enter and rewrite the earlier link.
  const closeAutocompleteIfOutside = completion.selectionChanged;
  onMount(() => {
    const owner = ref.ownerDocument;
    const syncNativeSelection = () => {
      if (owner.activeElement === ref) updateSel();
    };
    // Native selection may notify the document or textarea without select or
    // mouseup (Android WebView, GH #375). Capture both, but only update the
    // editor that owns focus.
    owner.addEventListener("selectionchange", syncNativeSelection, true);
    onCleanup(() => owner.removeEventListener("selectionchange", syncNativeSelection, true));
    selToolbar.follow(ref);
  });
  createEffect(() => {
    if (!selectionOverflowOpen() || !hasSel()) return;
    const unregister = registerTransientLayer({
      id: selectionOverflowLayerId,
      root: () => selectionOverflowRef ?? null,
      trigger: () => ref ?? null,
      dismiss: () => {
        const start = ref.selectionStart;
        const end = ref.selectionEnd;
        const direction = ref.selectionDirection;
        setSelectionOverflowOpen(false);
        queueMicrotask(() => {
          if (!ref.isConnected) return;
          ref.focus();
          ref.setSelectionRange(start, end, direction);
        });
        return true;
      },
    });
    onCleanup(unregister);
  });
  const runSelectionAction = (action: SelectionAction) => {
    applyEdit(action.apply(ref.value, ref.selectionStart, ref.selectionEnd, pageFmt(), ref.selectionDirection));
    setSelectionOverflowOpen(false);
    queueMicrotask(updateSel);
  };

  // A completed raw region maps back into its body-only code/calc projection.
  const replaceCompletion = (raw: string, caret: number, close: () => void) => {
    // Let a calculator completion enter calc mode in the mounted editor (GH #57).
    const enteredCalc = !editingCalc() ? calcSource(raw) : null;
    // A completion that lands a COMPLETE code wrapper (the language pick on the
    // opener line) swaps to the body-only view: map the caret from raw into body
    // space, the same transition as calc above.
    const codeWrap = enteredCalc === null ? codeBodyProjection(raw, pageFmt()) : null;
    commit(raw);
    if (enteredCalc !== null) setEditingCalc(true);
    close();
    if (codeWrap) codeView.enter(codeWrap.open.length);
    queueMicrotask(() => {
      const shown = enteredCalc ?? codeWrap?.body ?? raw;
      const openingEnd =
        enteredCalc !== null ? raw.indexOf("\n") + 1 : codeWrap ? codeWrap.open.length : 0;
      const shownCaret = enteredCalc !== null || codeWrap
        ? Math.max(0, Math.min(shown.length, caret - openingEnd))
        : caret;
      ref.value = shown;
      ref.setSelectionRange(shownCaret, shownCaret);
      ref.focus();
      autosize();
    });
  };

  const assets = editorAssets({
    id: props.id, element: () => ref, mounted: () => editorMounted, capture: !!cap,
    insert: (markdown, current) => {
      const start = ref.selectionStart, end = ref.selectionEnd;
      const newRaw = ref.value.slice(0, start) + markdown + ref.value.slice(end);
      commit(newRaw);
      const pos = start + markdown.length;
      queueMicrotask(() => {
        if (!current() || ref.value !== newRaw) return;
        ref.value = newRaw;
        ref.setSelectionRange(pos, pos);
        ref.focus();
        autosize();
      });
    },
  });
  const { uploadAsset, capturePhotoCmd, voiceMemoToggle, pasteClipboardFiles } = assets;

  const plainPaste = plainTextPaste();
  const clearPasteRaw = plainPaste.clearRaw;
  onCleanup(plainPaste.dispose);

  const pasteLiteralText = (text: string) => {
    const start = ref.selectionStart;
    const newRaw = ref.value.slice(0, start) + text + ref.value.slice(ref.selectionEnd);
    commit(newRaw);
    const pos = start + text.length;
    queueMicrotask(() => {
      syncTextareaToStore(ref, newRaw, textareaValue(), pos);
      autosize();
      refreshAutocompleteAfterInput();
    });
  };

  const capturePlainPaste = () => {
    const value = ref.value, start = ref.selectionStart, end = ref.selectionEnd;
    return {
      current: () => editorMounted && ref.isConnected && ref.ownerDocument.activeElement === ref
        && blockWritable(props.id) && !compositionActive && ref.value === value
        && ref.selectionStart === start && ref.selectionEnd === end,
      insert: pasteLiteralText,
    };
  };

  // Coalesce layout measurements; mount uses the immediate version.
  const resizeNow = () => resizeBlockEditor(ref);
  let autosizeRaf: (() => void) | undefined;
  const autosize = () => {
    if (autosizeRaf !== undefined) return; // already scheduled this frame
    autosizeRaf = requestFrame(ref, () => {
      autosizeRaf = undefined;
      resizeNow();
      pendingScrollAnchor?.restore();
      pendingScrollAnchor = undefined;
    });
  };

  const textareaValue = createMemo(() => isCalc() ? (calcLive() ?? "") : (codeShown()?.body ?? editorValue()));
  const editMixedContext = (event: MouseEvent, side: "before" | "after") => {
    const context = mixedCodeContext();
    if (!context || event.button !== 0 || forbidsEditEntry(event)) return;
    const raw = context[side];
    const offset = (side === "after" ? editorValue().length - raw.length : 0)
      + (renderedClickOffset(event.currentTarget as HTMLElement, raw, pageFmt(), event) ?? raw.length);
    event.preventDefault();
    event.stopPropagation();
    // The same parser-owned entry boundary handles another code card, or
    // returns to raw source for prose. No content is rewritten on a click.
    codeView.enter(offset);
    ref.value = textareaValue();
    const caret = Math.max(0, Math.min(ref.value.length, offset - (codeShown()?.open.length ?? 0)));
    ref.focus();
    ref.setSelectionRange(caret, caret);
    resizeNow();
  };
  // GH #668: document-driven changes (planning dates, undo, external updates)
  // bypass commit/onInput. Resize from the same visible buffer bound to the DOM,
  // after Solid has assigned it, using the existing coalesced layout frame.
  createEffect(() => { textareaValue(); if (ref) autosize(); });
  onCleanup(() => autosizeRaf?.());
  createEffect(() => { codeWrapping(); if (ref && codeEditing()) autosize(); });
  /** `again`: a later request to this mounted editor (see focusMountedEditor), applied, not taken. */
  const focusNow = (again?: { offset: CaretPos }) => {
    // After its block took a calculator's text (a template in the empty block) the editor enters calc
    // mode, as a fresh mount would.
    if (!editingCalc() && calcSource(editorValue()) !== null) setEditingCalc(true);
    const historySelection = again ? null : takeHistoryEditorSelectionFor(props.id, surfaceKey);
    const want = again ? again.offset : takeCaretFor(props.id);
    const rawCaret = historySelection ? historySelection.start
      : want !== null && typeof want === "object" && "start" in want ? want.codeBodyStart ?? null
      : typeof want === "number" ? want : null;
    if (rawCaret !== null) codeView.enter(rawCaret, historySelection?.end ?? rawCaret);
    ref.value = codeShown()?.body ?? (historySelection ? editorValue() : ref.value);
    ref.focus({ preventScroll: true }); // the engine's own focus scroll differs; revealCaretSettled (onMount) owns it (GH #745)
    const v = ref.value;
    if (historySelection) {
      const offset = codeShown()?.open.length ?? 0;
      const end = Math.min(historySelection.end - offset, v.length);
      const start = Math.min(historySelection.start - offset, end);
      ref.setSelectionRange(start, end);
      revealCaretColumn(ref, start);
      return;
    }
    if (want !== null && typeof want === "object" && "start" in want) {
      ref.setSelectionRange(want.start, want.end, want.direction);
      revealCaretColumn(ref, want.direction === "backward" ? want.start : want.end);
      return;
    }
    let offset: number;
    if (want == null) {
      offset = editorValue().length;
    } else if (typeof want === "number") {
      // Numeric targets arrive in RAW block coordinates. A body-only code editor
      // maps them through the wrapper: an opener hit snaps to the body start, a
      // closer/trailing hit to the body end.
      const p = codeShown();
      offset = p ? Math.max(0, Math.min(want - p.open.length, p.body.length)) : want;
    } else {
      // Cross-block navigation: Down targets the first source line; Up targets
      // the bottom visual row. The latter uses the mounted textarea's wrapping;
      // no-layout environments retain the old last-source-line fallback.
      if (want.edge === "first") {
        const nl = v.indexOf("\n");
        const lineLen = nl === -1 ? v.length : nl;
        offset = Math.min(want.col, lineLen);
      } else {
        const visualOffset = caretOffsetOnLastRow(ref, want.col);
        if (visualOffset !== null) offset = visualOffset;
        else {
          const lineStart = v.lastIndexOf("\n") + 1;
          offset = lineStart + Math.min(want.col, v.length - lineStart);
        }
      }
    }
    const o = Math.min(offset, v.length);
    ref.setSelectionRange(o, o);
    revealCaretColumn(ref, o);
  };
  onMount(() => {
    const unregisterHistoryTarget = registerHistoryEditorTarget({
      blockId: props.id,
      owner: editingOwner(),
      surface: surfaceKey,
      selection: () => codeView.selection(ref),
      viewport: () => ({ editor: ref, scroller: nearestScrollableY(ref) }),
      focused: () => !!ref && documentOf(ref).activeElement === ref,
    });
    onCleanup(unregisterHistoryTarget);
    focusMountedEditor(props.id, surfaceKey, () => ref, focusNow);
    resizeNow();
    onCleanup(revealCaretSettled(ref, nearestScrollableY(ref)));
    // A split can change the editor's wrapping width after this mount-time
    // measurement. Observe width only so the height write in `resizeNow` cannot
    // feed an observer loop; `autosize` keeps repeated layout changes to one
    // measurement per animation frame.
    let observedWidth = ref.clientWidth;
    const resizeObserver = newResizeObserver(ref, () => {
      const width = ref.clientWidth;
      if (width === observedWidth) return;
      observedWidth = width;
      autosize();
    });
    if (resizeObserver) {
      resizeObserver.observe(ref);
      onCleanup(() => resizeObserver.disconnect());
    }
  });

  // IMEs replace the textarea's transient composition range many times before
  // committing. Keep that range DOM-local: every setRaw() also dirties the page
  // and starts its save/reparse work. The finalized value has one canonical
  // commit at compositionend instead.
  let compositionActive = false;
  let compositionEndValue: string | null = null;
  // The composing text is not in the store yet, so a page reload from disk must
  // wait for it: hold the page for the composition (a deferred reload replays
  // when the hold is released). Ending the hold also ends the transaction: a
  // blur or teardown with no compositionend must not leave input swallowed.
  let releaseCompositionHold: (() => void) | null = null;
  const endCompositionHold = () => {
    compositionActive = false;
    releaseCompositionHold?.();
    releaseCompositionHold = null;
  };
  onCleanup(endCompositionHold);
  const beginComposition = () => {
    compositionActive = true;
    compositionEndValue = null;
    completion.pause();
    releaseCompositionHold ??= pinPageWhileDrafting(() => docNode(props.id)?.page ?? null);
  };
  const onInput = (e: InputEvent) => {
    // Some supported IMEs omit compositionstart but mark their composing input
    // (master f1c7de6e2). Enter the same transaction so the not-yet-committed
    // text stays DOM-local until compositionend.
    if (e.isComposing) {
      if (!compositionActive) beginComposition();
      return;
    }
    if (compositionActive) return;
    // Chromium-family engines can emit one ordinary input after compositionend.
    // Its DOM value has already committed above; suppress only that duplicate,
    // never a subsequent real edit with different text.
    const duplicate = compositionEndValue === ref.value;
    compositionEndValue = null;
    if (duplicate) return;
    // Editor keystroke post-processing on a single inserted char (not paste/IME/
    // delete). All branches edit ref.value BEFORE commit so the store sees it.
    if (e.inputType === "insertText" && e.data && e.data.length === 1 && !e.isComposing) {
      const ch = e.data;
      let handled = false;
      // OG parity: typing `::` at the beginning of a property line places the
      // caret before the delimiter. Subsequent property-name characters are
      // authored as `name|::`, not the malformed `::name` that caused GH #306.
      const propertyCaret = propertyNameInputCaret(ref.value, ref.selectionStart, ch);
      if (propertyCaret !== null) {
        ref.setSelectionRange(propertyCaret, propertyCaret);
        handled = true;
      }
      if (ch === "【" || ch === "（" || ch === "〈") handled = applyFullWidthRefReplace(ref);
      else if (ch === "】" || ch === "）") handled = applyFullWidthCloserThrough(ref, autoPairing());
      // GH #413: the third backtick of a whole-block fence trigger is an EXPLICIT
      // scaffold decision, not a character to pair: insert the matching closing
      // fence and offer the language picker on the still-visible opener line
      // (GH #507; the body-only view hides that line). Whole-buffer only, so a
      // backtick run inside prose keeps ordinary inline behaviour.
      if (
        !handled && ch === "`" && pageFmt() === "md" &&
        !isCalc() && codeShown() === null &&
        ref.value === "```" && ref.selectionStart === 3
      ) {
        const scaffold = "```\n\n```";
        ref.value = scaffold;
        commit(scaffold);
        openFenceLanguagePicker(scaffold, 3);
        return;
      }
      if (!handled && autoPairing()) {
        // Opt-in general auto-pairing (brackets/quotes), which also folds in the
        // `[[`/`((`/`{{` doubling so it composes with — not fights — page-ref pairing.
        const r = autoPairInsertOnInput(ref.value, ref.selectionStart, ch);
        if (r) {
          ref.value = r.value;
          ref.setSelectionRange(r.caret, r.caret);
          handled = true;
        }
      } else if (ch === "[" || ch === "]") {
        // Always-on OG-style page-ref pairing: `[[` → `[[]]`, type-through a `]`.
        const paired = autoPairEdit(ref.value, ref.selectionStart, ch);
        if (paired) {
          ref.value = paired.value;
          ref.setSelectionRange(paired.caret, paired.caret);
          handled = true;
        }
      }
      // "On type" typographic replacement (source gets the glyph). Pair chars and
      // typo triggers don't overlap, but skip if a pair op already consumed the char.
      if (!handled && !codeShown() && !isCalc() && typographyMode() === "type") {
        const typed = ref.value, fmt = pageFmt();
        const r = typoTypeReplace(typed, ref.selectionStart, ch, (from, to) => rangeInLiteral(typed, fmt, from, to));
        if (r) {
          ref.value = r.value;
          ref.setSelectionRange(r.caret, r.caret);
        }
      }
      // Only a whole visible block `1. ` becomes own numbered-list state.
      // Projected code/calc bodies are literal; their hidden wrappers must survive.
      // OG: src/main/frontend/handler/editor.cljs:1888-1892 (6e7afa8eb).
      if (ch === " " && ref.value === "1. " && !codeShown() && !isCalc() && makeOwnNumberedList(props.id, "")) {
        ref.value = "";
        ref.setSelectionRange(0, 0);
        autosize();
        refreshAutocompleteAfterInput();
        return;
      }
    }
    commit(ref.value);
    autosize();
    refreshAutocompleteAfterInput();
  };
  const onCompositionEnd = () => {
    compositionActive = false;
    if (!applyFullWidthRefReplace(ref)) applyFullWidthCloserThrough(ref, autoPairing());
    compositionEndValue = ref.value;
    commit(ref.value);
    endCompositionHold();
    autosize();
    refreshAutocompleteAfterInput();
  };

  // Move the block up/down among siblings, keeping edit mode + caret (the DOM
  // reorder briefly blurs the textarea; cross-day it remounts). A whole-block
  // embed's root moves its host on this page, like Tab (D-20261030-EMB).
  const moveBlockCmd = (e: KeyboardEvent, dir: 1 | -1): boolean => {
    e.preventDefault();
    const movedEditor = ref;
    const { id, scope } = position();
    const selection = { start: ref.selectionStart, end: ref.selectionEnd, direction: ref.selectionDirection, codeBodyStart: codeShown()?.open.length };
    const restore = () => restoreMovedSelection(ref, props.id, selection.start, selection.end, selection.direction);
    commit(ref.value);
    void withBlockMoving(docNode(id)?.page ?? "", async () => {
      startEditing(props.id, selection, null, editSurface());
      const intentIsCurrent = captureEditIntent();
      // A sibling reorder happens synchronously (a feed move's own sync part)
      // and keeps this textarea. Restore it in the same gesture: waiting a
      // frame lets Android dismiss the IME despite the later focus.
      const move = scope && !scope.navOnly ? moveItem(id, dir) : moveBlockFeed(id, dir);
      if (intentIsCurrent() && ref === movedEditor && movedEditor.isConnected && editingId() === props.id
        && (documentOf(movedEditor).activeElement === movedEditor
          || documentOf(movedEditor).activeElement === documentOf(movedEditor).body)) restore();
      const moved = await move;
      // GH #641: a move this surface cannot show says where the block went.
      if (moved !== false && moved !== "none") noticeMoveOutOfView(id, scope);
      await nextFrame(ref);
      if (intentIsCurrent() && documentOf(ref).activeElement !== ref) restore();
    }).catch(() => console.error("Block move failed"));
    return true;
  };
  // Shift+Up/Down: start a block selection only when the caret is on the block's
  // first/last VISUAL row (source-`\n` pre-filter + visual-row check, so a long
  // wrapped line isn't treated as one line). Off the edge → return false so the
  // textarea extends the selection by a wrapped line natively.
  const selectBlockCmd = (e: KeyboardEvent, dir: 1 | -1): boolean => {
    const raw = ref.value;
    // Test the ACTIVE end of the selection (the one Shift+Arrow moves), not the
    // anchor: when a selection is extended down through a multiline block, the
    // caret is at selectionEnd while selectionStart stays put up top — using the
    // anchor meant a multiline block never reached the "last row" test, so it
    // never switched from text-selection to block-selection (a single-line block
    // happened to work because anchor == caret).
    const caret = ref.selectionDirection === "backward" ? ref.selectionStart : ref.selectionEnd;
    const atEdge =
      dir > 0
        ? !raw.slice(caret).includes("\n") && caretAtLastRow(ref, caret)
        : !raw.slice(0, caret).includes("\n") && caretAtFirstRow(ref, caret);
    if (!atEdge) return false;
    e.preventDefault();
    commit(raw);
    selectBlock(position().id, position().scope);
    moveSelection(dir, true);
    return true;
  };
  const cycleTodoCmd = () => {
    const start = ref.selectionStart;
    const { raw: newRaw, delta } = cycleMarkerSmart(ref.value, workflow(), pageFmt());
    commit(newRaw);
    const pos = Math.max(0, start + delta);
    queueMicrotask(() => {
      syncTextareaToStore(ref, newRaw, textareaValue(), pos);
      autosize();
    });
  };
  const softNewlineCmd = () => {
    const start = ref.selectionStart;
    const end = ref.selectionEnd;
    closeAc();
    applyEdit({ text: ref.value.slice(0, start) + "\n" + ref.value.slice(end), start: start + 1, end: start + 1 });
  };
  const insertPairedRefTrigger = (open: "[[" | "((", close: "]]" | "))") => {
    const start = ref.selectionStart;
    const end = ref.selectionEnd;
    const selected = ref.value.slice(start, end);
    const insert = `${open}${selected}${close}`;
    const caret = start + open.length + selected.length;
    applyEdit({ text: ref.value.slice(0, start) + insert + ref.value.slice(end), start: caret, end: caret });
    queueMicrotask(() => void updateAutocomplete());
  };
  const insertSlashMenuTrigger = () => {
    const start = ref.selectionStart;
    const end = ref.selectionEnd;
    applyEdit({ text: ref.value.slice(0, start) + "/" + ref.value.slice(end), start: start + 1, end: start + 1 });
    queueMicrotask(() => void updateAutocomplete());
  };
  // GH #448 "Paste as link": the clipboard's URL or block ref becomes a link
  // around the selection (a bare link without one). A command, so it reads the
  // clipboard itself; ordinary paste is untouched.
  const pasteAsLinkCmd = async () => {
    const before = { value: ref.value, start: ref.selectionStart, end: ref.selectionEnd };
    let clipboard: string;
    try {
      clipboard = await readClipboardText();
    } catch {
      pushToast(msg("editor-block-couldn-t-read-the-clipboard"), "error");
      return;
    }
    // The user kept typing while the clipboard was read: the request is stale.
    if (!ref.isConnected || ref.value !== before.value) return;
    const edit = pasteAsLinkEdit(before.value, before.start, before.end, clipboard, pageFmt());
    if (!edit) {
      pushToast(msg("editor-block-the-clipboard-doesn-t-hold-a-link-or-a-block-ref"), "warn");
      return;
    }
    applyEdit(edit);
  };
  const openScheduledDatePicker = () => {
    const r = ref.getBoundingClientRect();
    openDatePicker(props.id, "scheduled", r.left, r.bottom + 4, resultTarget(props.id));
  };

  const applyInlineFormat = (kind: InlineFormat) => {
    applyEdit(toggleInlineFormat(
      ref.value,
      ref.selectionStart,
      ref.selectionEnd,
      pageFmt(),
      kind,
      ref.selectionDirection,
    ));
  };

  // Editor command handlers keyed by command id (see keybindings.ts). Each does
  // its own preventDefault when it handles the event and returns whether it did
  // (false → fall through to native handling). Read ref fresh at call time.
  const runEditorCmd: Record<string, (e: KeyboardEvent) => boolean> = {
    ...Object.fromEntries(Object.entries(textCommandHandlers({
      text: () => ref.value, start: () => ref.selectionStart, apply: applyEdit, move: moveCaret,
    })).map(([id, run]) => [id, (e: KeyboardEvent) => { e.preventDefault(); return run(); }])),
    "editor/insert-youtube-timestamp": (e) => { e.preventDefault(); insertYoutubeTimestamp(ref, applyEdit); return true; },
    "editor/select-embedding-block": (e) => {
      if (!embedNavExit) return false;
      e.preventDefault();
      commit(ref.value);
      // The source's outline scope belongs to the embed. Selection belongs to
      // its containing outline instead (including a nested embed's host).
      const host = ref.closest(".embed-block, .embed-page")?.closest<HTMLElement>(".ls-block");
      selectBlock(embedNavExit.hostBlockId);
      host?.querySelector<HTMLElement>(":scope > .block-main")?.scrollIntoView?.({ block: "nearest" });
      return true;
    },
    "editor/bold": (e) => { e.preventDefault(); applyInlineFormat("bold"); return true; },
    "editor/italics": (e) => { e.preventDefault(); applyInlineFormat("italic"); return true; },
    "editor/strike-through": (e) => { e.preventDefault(); applyInlineFormat("strikethrough"); return true; },
    "editor/highlight": (e) => { e.preventDefault(); applyInlineFormat("highlight"); return true; },
    "editor/insert-link": (e) => { e.preventDefault(); applyEdit(insertLink(ref.value, ref.selectionStart, ref.selectionEnd, pageFmt())); return true; },
    // GH #279: embed twin of Mod+C; with a text selection decline so the platform copy runs.
    "editor/copy-embed": (e) => { if (ref.selectionStart !== ref.selectionEnd) return false; e.preventDefault(); commit(ref.value); void copyBlockLink(props.id, "embed"); return true; },
    // Margin dialogue (vision §3.7). Always consumed: Ctrl/Cmd+R must not reach
    // the webview, whose default is reload. The capture window shows one block.
    "editor/comment": (e) => { e.preventDefault(); e.stopPropagation(); if (!cap) commentFromEditor(props.id, ref, commit, surfaceKey, sheetCell && editsLikeOutline(sheetCell) ? cellOwner(sheetCell) : null); return true; },
    "editor/move-block-up": (e) => moveBlockCmd(e, -1),
    "editor/move-block-down": (e) => moveBlockCmd(e, 1),
    "editor/collapse": (e) => { e.preventDefault(); setOccurrenceCollapsed(true); return true; },
    "editor/expand": (e) => { e.preventDefault(); setOccurrenceCollapsed(false); return true; },
    "editor/select-block-up": (e) => selectBlockCmd(e, -1),
    "editor/select-block-down": (e) => selectBlockCmd(e, 1),
    "editor/select-all": (e) => {
      // GH #262: first press keeps the native select-all-text behaviour
      // (return false without preventDefault). A press with the text already
      // fully selected escalates to a block-level subtree selection; later
      // presses climb ancestors in selection mode (keybindings.ts).
      const fullySelected = ref.selectionStart === 0 && ref.selectionEnd === ref.value.length;
      if (!fullySelected) return false;
      e.preventDefault();
      commit(ref.value);
      selectBlockSubtree(position().id, position().scope);
      return true;
    },
    "editor/cycle-todo": (e) => { e.preventDefault(); cycleTodoCmd(); return true; },
    "editor/new-sibling-after-subtree": (e) => {
      e.preventDefault();
      const created = insertSiblingAfterSubtree(props.id, () => commit(ref.value), outlineScope?.forceExpandedRoot === props.id);
      if (created) startEditing(created, 0, null, editSurface());
      return true;
    },
    "editor/indent": (e) => {
      e.preventDefault();
      // First-line Markdown markers keep OG outline Tab semantics (GH #632).
      // Later in-block list lines and Org retain their list-item nudges.
      const ll = listLine(ref.value, ref.selectionStart);
      if (ll && (pageFmt() === "org" || ll.lineStart > 0)) { nudgeListItem(ll, +2); return true; }
      const { id, scope } = position();
      if (!gridNestedBlock(props.id, sheetCell) && !scope?.navOnly && scope?.roots.includes(id)) return true;
      const selection = { start: ref.selectionStart, end: ref.selectionEnd, direction: ref.selectionDirection, codeBodyStart: codeShown()?.open.length };
      commit(ref.value);
      if (indentBlock(id, selection, editSurface(), props.id) === false) pushToast(msg("editor-block-outline-is-too-deep-to-indent"), "error");
      return true;
    },
    "editor/outdent": (e) => {
      e.preventDefault();
      const ll = listLine(ref.value, ref.selectionStart);
      if (ll && (pageFmt() === "org" || ll.lineStart > 0) && ll.indent.length > 0) { nudgeListItem(ll, -2); return true; }
      const { id, scope } = position();
      if (scope?.forceExpandedRoot === docNode(id)?.parent) return true;
      const selection = { start: ref.selectionStart, end: ref.selectionEnd, direction: ref.selectionDirection, codeBodyStart: codeShown()?.open.length };
      commit(ref.value); outdentBlock(id, selection, editSurface(), props.id); return true;
    },
  };
  const mobileKeyEvent = { preventDefault() {} } as KeyboardEvent;
  const dispatchMobileEditorCommand = (command: MobileEditorCommandId): boolean => runGridOutline(sheetCell, () => {
    if (!ref || !ref.isConnected) return false;
    if (sheetCell && editsLikeOutline(sheetCell) && gridBlocksCommand(props.id, sheetCell, command)) return true;
    ref.focus();
    switch (command) {
      case "editor/outdent":
      case "editor/indent":
      case "editor/move-block-up":
      case "editor/move-block-down":
      case "editor/select-embedding-block":
      case "editor/cycle-todo":
        return runEditorCmd[command]?.(mobileKeyEvent) ?? false;
      case "editor/soft-newline":
        softNewlineCmd();
        return true;
      case "editor/upload-asset":
        uploadAsset();
        return true;
      case "editor/capture-photo":
        void capturePhotoCmd();
        return true;
      case "editor/voice-memo":
        void voiceMemoToggle();
        return true;
      case "editor/open-date-picker":
        openScheduledDatePicker();
        return true;
      case "editor/insert-page-ref":
        insertPairedRefTrigger("[[", "]]");
        return true;
      case "editor/insert-block-ref":
        insertPairedRefTrigger("((", "))");
        return true;
      case "editor/open-slash-menu":
        insertSlashMenuTrigger();
        return true;
      case "editor/paste-as-link":
        void pasteAsLinkCmd();
        return true;
      case "editor/paste-plain":
        plainPaste.request(capturePlainPaste);
        return true;
    }
  });
  let unregisterFocusedEditorBridge: (() => void) | undefined;
  const registerFocusedEditorBridge = () => {
    unregisterFocusedEditorBridge?.();
    unregisterFocusedEditorBridge = registerFocusedEditorCommandBridge({
      blockId: props.id,
      dispatch: dispatchMobileEditorCommand,
      blur: () => ref.blur(),
      textSelection: () => (ref && ref.selectionStart !== ref.selectionEnd
        ? { text: ref.value, start: ref.selectionStart, end: ref.selectionEnd }
        : null),
    });
  };
  const unregisterFocusedEditor = () => {
    unregisterFocusedEditorBridge?.();
    unregisterFocusedEditorBridge = undefined;
  };
  onCleanup(unregisterFocusedEditor);
  let sheetCanceling = false;

  const sheetVisibleLength = (id: string): number =>
    splitProps(docNode(id)?.raw ?? "", SHEET_CELL_HIDE).visible.length;

  const handleSheetCellKey = (e: KeyboardEvent, start: number, end: number, raw: string): boolean => {
    if (!sheetCell) return false;
    if (editsLikeOutline(sheetCell)) {
      const leave = arrowLeave(e.key, ref, start, end, raw.length);
      const up = e.key === "ArrowUp" && !raw.slice(0, start).includes("\n") && caretAtFirstRow(ref, start);
      const down = e.key === "ArrowDown" && !raw.slice(start).includes("\n") && caretAtLastRow(ref, start);
      const sourceCol = start - (raw.slice(0, start).lastIndexOf("\n") + 1);
      const handled = gridEditKey(e, { id: props.id, cell: sheetCell, raw, start, end, commit,
        softNewline: softNewlineCmd,
        edge: leave ? (leave === "forward" ? "right" : "left") : up ? "up" : down ? "down" : null,
        col: up || down ? caretColumnOnVisualRow(ref, start) ?? sourceCol : 0 });
      if (handled) e.preventDefault();
      return handled;
    }
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    const commitAndSelect = () => {
      commit(raw);
      selectCellAfterEdit(sheetCell);
    };
    const commitAndMove = (dir: "up" | "down" | "left" | "right" | "tab-forward" | "tab-back") => {
      commit(raw);
      moveCellAfterEdit(sheetCell, dir);
    };
    const commitAndStartSheetEdit = (id: string, offset: number) => {
      startEditing(id, offset, cellOwner(sheetCell));
    };
    const commitAndAscend = () => {
      commit(raw);
      const hostId = cellBlockId(sheetCell);
      if (hostId && props.id !== hostId) {
        const prev = sheetOutlineNeighbor(props.id, hostId, -1);
        if (prev) commitAndStartSheetEdit(prev, sheetVisibleLength(prev));
        else moveCellAfterEdit(sheetCell, "up");
        return;
      }
      moveCellAfterEdit(sheetCell, "up");
    };
    const commitAndDescend = () => {
      const nestedGridId = sheetFaceGridId(props.id);
      commit(raw);
      if (nestedGridId) selectTopRowSeamAfterEdit(
        nestedGridId,
        0,
        sheetCell ? cellSurfaceKey(sheetCell.gridId, sheetCell.surfaceId) : undefined
      );
      else {
        const hostId = cellBlockId(sheetCell);
        const next = hostId
          ? sheetOutlineNeighbor(props.id, hostId, 1)
          : null;
        if (next) commitAndStartSheetEdit(next, 0);
        else moveCellAfterEdit(sheetCell, "down");
      }
    };
    const commitAndAppendChild = () => {
      commit(raw);
      const hostId = cellBlockId(sheetCell) ?? props.id;
      const childId = appendSheetCellChild(hostId);
      if (childId) commitAndStartSheetEdit(childId, 0);
    };

    if (!e.ctrlKey && !e.metaKey && e.altKey && e.key === "Enter") {
      e.preventDefault();
      commitAndAppendChild();
      return true;
    }
    if (plain && e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      commitAndSelect();
      return true;
    }
    if (isPermittedTabGesture(e)) {
      e.preventDefault();
      commitAndMove(e.shiftKey ? "tab-back" : "tab-forward");
      return true;
    }
    if (plain && e.key === "Escape") {
      e.preventDefault();
      sheetCanceling = true;
      if (sheetInitialRaw !== null && node().raw !== sheetInitialRaw) {
        setRaw(props.id, sheetInitialRaw, { timetracking: false });
      }
      selectCellAfterEdit(sheetCell);
      return true;
    }
    if (plain && e.key === "Backspace" && start === 0 && end === 0) {
      e.preventDefault();
      return true;
    }
    if (plain && !e.shiftKey && arrowLeave(e.key, ref, start, end, raw.length)) {
      e.preventDefault();
      commitAndMove(e.key === "ArrowLeft" ? "left" : "right");
      return true;
    }
    if (plain && !e.shiftKey && start === end && e.key === "ArrowUp") {
      const before = raw.slice(0, start);
      if (!before.includes("\n") && caretAtFirstRow(ref, start)) {
        e.preventDefault();
        commitAndAscend();
        return true;
      }
    }
    if (plain && !e.shiftKey && start === end && e.key === "ArrowDown") {
      const after = raw.slice(start);
      if (!after.includes("\n") && caretAtLastRow(ref, start)) {
        e.preventDefault();
        commitAndDescend();
        return true;
      }
    }
    return false;
  };

  const onKeyDown = (e: KeyboardEvent) => runGridOutline(sheetCell, () => handleOutlineKey(e));
  const handleOutlineKey = (e: KeyboardEvent) => {
    // IME owns its key events until compositionend commits the finalized value.
    if (compositionActive || e.isComposing || e.keyCode === 229) return;

    if (codeView.leaveOnKey(e, ref)) { autosize(); return; }
    const start = ref.selectionStart;
    const end = ref.selectionEnd;
    const raw = ref.value;
    // Empty code payloads can leave their wrapper through the existing delete door.
    if (e.key === "Backspace" && !e.ctrlKey && !e.metaKey && !e.altKey && !sheetCell && codeShown() && raw === "" && node().children.length === 0 && bodyScopeOf(props.id) !== undefined) {
      e.preventDefault();
      const adjacent = [bodyNeighbor(props.id, -1, structuralScope), bodyNeighbor(props.id, 1, structuralScope)].find(id => id && docNode(id)?.page === node().page);
      if (adjacent) {
        deleteBlock(props.id);
        startEditing(adjacent, 0, null, editSurface());
      } else { setRaw(props.id, "", { timetracking: false }); }
      return;
    }

    if (plainPaste.keydown(e, capturePlainPaste)) return;

    if (acVisible() && e.key === "Escape") {
      e.preventDefault(); closeAc(); return;
    }
    // Autocomplete popup takes priority for navigation/selection keys.
    if (ac() && acItems().length) {
      const n = acItems().length;
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setAcIndex((acIndex() + 1) % n);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setAcIndex((acIndex() - 1 + n) % n);
        return;
      }
      if (e.key === "Enter" || isPermittedTabGesture(e)) {
        e.preventDefault();
        selectAc(acItems()[acIndex()]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        closeAc();
        return;
      }
    }

    if (commentKey(e, props.id, softNewlineCmd, () => { commit(ref.value); endEdit("page-navigation"); })) return;
    if (handleSheetCellKey(e, start, end, raw)) return;

    // Raw Control is not represented by `mod` on macOS. Decline every
    // modified Tab before command lookup so it cannot become indent/outdent.
    if (isTabLikeEvent(e) && !isPermittedTabGesture(e)) return;

    // Resolve configured editor commands before incidental literal-key behavior:
    // an explicit user binding (including Alt+[) must win over selection wrapping.
    const cmd = editorCommandFor(e);

    // Auto-pair wrap on a SELECTION (OG parity, always-on — independent of the
    // opt-in empty-caret auto-pairing). Typing any of `SELECTION_WRAP` around
    // selected text wraps it, keeping the selection: `*`/`~`/`=` etc. so a second
    // press gives `**bold**`/`~~strike~~`/`==highlight==`, and `[`/`(` so `[[sel]]`
    // makes a page ref and `((sel))` a block ref — the doubling bracket then opens
    // the matching search seeded with the selection, so Enter links it to an
    // existing page/block or creates it. Match OG by accepting an Alt-modified
    // event when the layout still reports the literal delimiter as `event.key`;
    // layout-produced characters remain native because they are not in the wrap
    // map. Never shadow Ctrl/Cmd commands, explicit editor bindings, or IME input.
    if (
      start !== end &&
      !cmd && !e.ctrlKey && !e.metaKey && !e.isComposing &&
      e.key.length === 1 && Object.prototype.hasOwnProperty.call(SELECTION_WRAP, e.key)
    ) {
      const ed = wrapSelectionEdit(raw, start, end, e.key);
      if (ed) {
        e.preventDefault();
        applyEdit(ed);
        // The bracket that just DOUBLED (`[[sel]]` / `((sel))`) opens the page/
        // block search seeded with the selection. applyEdit writes the textarea in
        // a microtask, so defer to a following one (FIFO): collapse the caret to
        // the inner end so detectTrigger reads the selection as the query, then
        // open the popup. Enter picks an existing page/block or creates it.
        if (doubleRefKind(ed.text, ed.start, ed.end)) {
          queueMicrotask(() => {
            ref.setSelectionRange(ed.end, ed.end);
            void updateAutocomplete();
          });
        }
        return;
      }
    }

    // Edit-mode Mod+C with NO text selected → copy a reference to this block
    // (`((uuid))`), matching OG. With a selection, fall through to the browser's
    // normal text copy. Persist current edits first so the id:: lands durably.
    if (
      (e.ctrlKey || e.metaKey) &&
      !e.shiftKey &&
      !e.altKey &&
      shortcutKey(e) === "c" &&
      start === end
    ) {
      e.preventDefault();
      commit(raw);
      void copyBlockLink(props.id, "ref");
      return;
    }

    // Configurable editor commands → one dispatch through the handler table
    // (runEditorCmd) instead of ~20 sequential matchesCommand checks. A handler
    // returns false to fall through — select-block does this off the block edge
    // so the textarea extends the selection by a wrapped line.
    if (sheetCell && cmd && (editsLikeOutline(sheetCell) ? gridBlocksCommand(props.id, sheetCell, cmd) : SHEET_CELL_BLOCKED_EDITOR_COMMANDS.has(cmd))) {
      e.preventDefault();
      return;
    }
    if (cmd && runEditorCmd[cmd]?.(e)) return;

    // Quick-capture window key handling (cap set only there). Runs AFTER the
    // autocomplete-popup block above, so when the popup is open Enter still
    // selects the highlighted item — only a popup-closed Enter files.
    const captureIntent = captureKeyIntent(e, cmd, cap);
    if (captureIntent && cap) {
      e.preventDefault();
      if (captureIntent === "submit") { commit(raw); cap.submit(); }
      else cap.cancel();
      return;
    }

    // OG maps document-mode Enter to newline and Shift+Enter to new-block before
    // it reaches the existing new-block handler, unless its escape hatch is set
    // (`src/main/frontend/handler/editor.cljs:2521-2533`,
    // `src/main/frontend/state.cljs:714-717` at `6e7afa8eb`).
    // Select between the existing mutation paths here; the structural branch
    // below intentionally retains every current code-fence/list/etc. exception.
    const docModeEnterForNewLine = documentMode() && !docModeEnterForNewBlock();
    const mappedToSoftNewline =
      e.key === "Enter" && !e.ctrlKey && !e.metaKey && !e.altKey && (
        (!docModeEnterForNewLine && e.shiftKey) || (docModeEnterForNewLine && !e.shiftKey)
      );
    if (mappedToSoftNewline) {
      e.preventDefault();
      softNewlineCmd();
      return;
    }

    if (
      e.key === "Enter" && !e.ctrlKey && !e.metaKey &&
      (!e.shiftKey || (docModeEnterForNewLine && !e.altKey))
    ) {
      // A folded embed's displayed root is the host's keyboard boundary. Use
      // occurrence collapse, not source collapse; source editors and descendants
      // retain their ordinary structural Enter behavior (GH #642).
      const foldedEmbedHost = position().id !== props.id && collapseSurface?.collapsed(props.id, node().collapsed)
        ? position().id : null;
      if (foldedEmbedHost && start === end && end === raw.length) {
        e.preventDefault();
        commit(raw);
        const newId = insertOutlineAfter(foldedEmbedHost, [{ raw: "", children: [] }]);
        if (newId) startEditing(newId, 0);
        return;
      }
      const inFence = !isAnnot() && caretInFence(raw, start, pageFmt());
      // GH #278: a multi-line `$$ … $$` environment behaves like a fence for
      // Enter. See caretInDisplayMath — a deliberate divergence from OG.
      const inMath = !isAnnot() && !inFence && caretInDisplayMath(raw, start, pageFmt());
      const inPageProperties = !isAnnot() && isFirstPagePropertiesBlock(raw);
      // GH #412/#413: a body-only code editor has no fence lines in view, so
      // `caretInFence` cannot see it; gate on the projection directly.
      const inCode = !isAnnot() && codeShown() !== null;
      // Double-Enter escape: the first Enter creates a trailing blank line; the
      // second removes that sentinel and creates a normal sibling. Keep the text
      // trim and structural insertion in one undo unit so one Undo restores the
      // exact pre-exit special block and removes the sibling.
      if ((isCalc() || inFence || inMath || inPageProperties || inCode) && start === end) {
        const kind = isCalc() ? "calc" : inFence ? "fence" : inMath ? "math" : inCode ? "code" : "properties";
        const trimmed = kind === "code" ? codeBodyExitTrim(raw, start) : multilineExitTrim(raw, start, kind, pageFmt());
        if (trimmed !== null) {
          e.preventDefault();
          // One gesture: the kernel is asked about the sibling over the trimmed text, which is written only
          // once it has answered, so a refused or failed insert leaves the blank line in place.
          const newId = exitMultilineBlock(props.id, stage(trimmed));
          if (newId) startEditing(newId, 0, null, editSurface());
          return;
        }
      }
      // A Markdown page's first properties-only bullet is OG's page-property
      // editor. Enter after a property stays in the same textarea so the next
      // `key:: value` pair can be typed; Enter again on the empty trailing line
      // takes the double-Enter exit above and creates an ordinary body bullet.
      // Use the explicit edit path instead of relying on a browser textarea's
      // native default so the update/autosize/undo behavior is deterministic.
      if (inPageProperties && caretOnPropertyLine(raw, start)) {
        e.preventDefault();
        softNewlineCmd();
        return;
      }
      // In a calc block, Enter adds a new expression line (stays in the block) —
      // let the textarea insert the newline natively, like OG.
      if (isCalc()) return;
      e.preventDefault();
      // Inside a body-only code editor, Enter inserts a real newline and stays in
      // the block (same rule as the raw-fence path below it).
      if (inCode) {
        softNewlineCmd();
        return;
      }
      // Inside a fenced code block, Enter inserts a real newline and stays in the
      // block instead of splitting into a new bullet (which would break the fence
      // — GH #66). caretInFence treats a still-unterminated fence (being typed) as
      // inside too, and returns false when the caret sits on a ``` delimiter line,
      // so Enter on the closing fence still exits the block.
      if (!isAnnot() && (inFence || inMath || caretOnOpeningFence(raw, start, pageFmt()))) {
        softNewlineCmd();
        return;
      }
      if (!isAnnot() && stopOwnNumberedListOnEmptyEnter(props.id, raw)) {
        ref.setSelectionRange(0, 0);
        autosize();
        return;
      }
      // In-block list: Enter on a `+`/`*`/ordered list line CONTINUES the list
      // (new item below, same marker/indent; a checkbox item starts a fresh `[ ]`)
      // instead of splitting the block. To exit, Backspace the empty item down to a
      // blank line, then Enter on that non-list line makes a new bullet.
      const ll = !isAnnot() ? listLine(raw, start) : null;
      if (ll) {
        const ordered = /\d/.test(ll.marker);
        const nextMarker = ordered ? parseInt(ll.marker) + 1 + ll.marker.replace(/\d+/, "") : ll.marker;
        const prefix = ll.indent + nextMarker + " " + (ll.hasCheckbox ? "[ ] " : "");
        const caret = start + 1 + prefix.length;
        // applyEdit (not commit+startEditing) so the textarea re-autosizes — else
        // the new line is clipped until the next keystroke.
        applyEdit({ text: raw.slice(0, start) + "\n" + prefix + raw.slice(start), start: caret, end: caret });
        return;
      }
      // Enter over a selection deletes the selection, then splits at its start, as OG does (6e7afa8eb
      // editor.cljs `compute-fst-snd-block-text`) and as the CodeMirror editor does. It is one gesture:
      // the kernel splits the text the selection leaves, and the editor and the block lose the selected
      // text only once it has answered, so a refusal or a parser failure leaves both as they were.
      // A highlight block keeps its text whole (it is never split).
      const replacement = start === end || isAnnot() ? undefined : deleteRange(start, end);
      if (!replacement) commit(raw); // flush current text
      if (isAnnot()) {
        // A highlight block isn't split (that would mangle its metadata); Enter
        // adds a new sibling bullet below, which the user can Tab to nest as a
        // note under the highlight.
        const newId = insertOutlineAfter(props.id, [{ raw: "", children: [] }]);
        if (newId) startEditing(newId, 0, null, editSurface());
      } else {
        splitBlockInView(props.id, start, outlineScope, navigationSurface(), hideFn(), replacement, collapseSurface);
      }
    } else if (e.key === "Backspace" && end === start) {
      // Auto-pair Backspace: caret between an empty pair (`(|)`) deletes both
      // chars, so an unwanted auto-inserted closer clears in one press. General
      // auto-pairing is opt-in, but the page-ref pairing (`[[`→`[[]]`) is
      // ALWAYS on — so the bracket case must clean up even with the opt-in off,
      // otherwise backspacing a `[[]]` strands `]]` (GH #19).
      {
        const ed = backspacePairEdit(raw, start);
        if (ed && (autoPairing() || (raw[start - 1] === "[" && raw[start] === "]"))) {
          e.preventDefault();
          applyEdit(ed);
          return;
        }
      }
      // In-block list: Backspace at the head of a list item's text removes the
      // marker (turns it into a blank/plain line) — the way to exit the list.
      const ll = listLine(raw, start);
      if (ll && start === ll.lineStart + ll.prefixLen) {
        e.preventDefault();
        applyEdit({ text: raw.slice(0, ll.lineStart) + raw.slice(start), start: ll.lineStart, end: ll.lineStart });
        return;
      }
      if (start === 0) {
        // Never merge a highlight, calc, or body-only code block away (their
        // structure must stay; a merge would smuggle raw wrapper bytes into the
        // previous block's text).
        if (isAnnot() || isCalc() || codeShown() || bodyScopeOf(props.id) === undefined) return;
        // Own-numbered state is a block property, never an in-block `1.` marker.
        // At offset zero OG removes only that property and preserves the text
        // (`src/main/frontend/handler/editor.cljs:2752-2764`, 6e7afa8eb).
        if (removeOwnNumberedList(props.id)) {
          e.preventDefault();
          ref.setSelectionRange(0, 0);
          autosize();
          return;
        }
        commit(raw);
        if (mergeWithPrev(props.id, structuralScope, editSurface())) {
          e.preventDefault();
          return;
        }
        const n = docNode(props.id);
        const next = bodyNeighbor(props.id, 1, structuralScope);
        if (n && isEditorEmpty(n.raw, hideFn(), pageFmt()) && n.children.length === 0 && next && docNode(next)?.page === n.page) {
          e.preventDefault();
          deleteBlock(props.id);
          startEditing(next, 0, null, editSurface());
        }
      }
    } else if (e.key === "Delete" && end === start && start === raw.length) {
      // Delete merges with the next body block (GH #213, #891); annotations,
      // calc and code wrappers keep their structure, as on Backspace.
      if (isAnnot() || isCalc() || codeShown() || bodyScopeOf(props.id) === undefined) return;
      const next = bodyNeighbor(props.id, 1, structuralScope);
      if (next) {
        const nextRaw = docNode(next)?.raw ?? "";
        if (isAnnotationBlock(nextRaw, pageFmt()) || calcSource(nextRaw) !== null) return;
        commit(raw);
        if (mergeWithNext(props.id, structuralScope, editSurface())) {
          e.preventDefault();
          const caretAt = start; // join point = the block's pre-merge end
          queueMicrotask(() => {
            ref.setSelectionRange(caretAt, caretAt);
            autosize();
          });
        }
      }
    } else if ((e.key === "ArrowLeft" || e.key === "ArrowRight") && !e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey) {
      // GH #213: past the block's start, into the END of the previous visible
      // editor; past its end, into the START of the next. Shift keeps native
      // selection; Ctrl/Meta keep native word/line jumps. GH #823: which key that
      // is depends on the block's direction.
      const leave = arrowLeave(e.key, ref, start, end, raw.length);
      if (leave === "backward") {
        const prev = viewPrev();
        if (prev) {
          e.preventDefault();
          // A number caret clamps to the new editor's full text length at mount.
          startEditing(prev, Number.MAX_SAFE_INTEGER, null, navigationSurface());
        }
      } else if (leave === "forward") {
        const next = viewNext();
        if (next) {
          e.preventDefault();
          startEditing(next, 0, null, navigationSurface());
        }
      }
    } else if (e.key === "ArrowUp" && !e.shiftKey) {
      // Leave for the previous block only from the FIRST visual row; otherwise
      // let the textarea move the caret up one wrapped line. (A long single line
      // has no `\n` but still wraps — the source-`\n` test alone wrongly jumped
      // to the parent from the second visual row.)
      const before = raw.slice(0, start);
      if (!before.includes("\n") && caretAtFirstRow(ref, start)) {
        let prev = viewPrev();
        // GH #415: at the top of an embed ROOT there is no in-surface previous
        // block. Exit upward to the block preceding the embed on the host page
        // (editing the host itself would unmount the embed under the caret); the
        // destination mounts in the primary surface, not this embed surface.
        // og has no embed-scoped outline (master GH #341), so the source page's
        // predecessor of the embed root is not "none": the root itself is the test.
        const exitingEmbed = embedNavExit !== null && (!prev || embedNavExit.firstRoot() === props.id);
        if (exitingEmbed) prev = bodyNeighbor(embedNavExit!.hostBlockId, -1);
        // A canonical page header is not normally an outline node. Materialize
        // its transient ordinary-editor representation only when the primary
        // page/pane caret crosses the first-body boundary; reference, embed and
        // right-sidebar copies must never synthesize it.
        if (
          !prev && !outlineScope
          && (surfaceKey === "main" || surfaceKey.startsWith("pane:"))
          && pageByName(node().page)?.roots[0] === props.id
        ) {
          prev = beginPageHeaderEdit(node().page);
        }
        if (prev) {
          e.preventDefault();
          // Keep the caret's column on the previous block's bottom visual row.
          // Resolution happens after its textarea mounts, when wrapping is known.
          const entry = !exitingEmbed ? gridEntry(prev, "below", caretColumnOnVisualRow(ref, start) ?? start - (before.lastIndexOf("\n") + 1), surfaceKey) : null;
          if (entry) { commit(raw); openGridTarget(entry, navigationSurface() ?? undefined); return; }
          startEditing(prev, { col: start - (before.lastIndexOf("\n") + 1), edge: "last" }, null, exitingEmbed ? null : navigationSurface());
        }
      }
    } else if (e.key === "ArrowDown" && !e.shiftKey) {
      const after = raw.slice(start);
      if (!after.includes("\n") && caretAtLastRow(ref, start)) {
        // OG parity: keep the caret's VISUAL column — land it that many chars
        // into the first source line of the next block (clamped). A wrapped
        // source line can contain several visual rows; using its source column
        // here would jump to the end of the next block.
        const sourceCol = start - (raw.slice(0, start).lastIndexOf("\n") + 1);
        const col = caretColumnOnVisualRow(ref, start) ?? sourceCol;
        const entry = gridEntry(props.id, "above", col, surfaceKey);
        if (entry) { e.preventDefault(); commit(raw); openGridTarget(entry, navigationSurface() ?? undefined); return; }
        const next = viewNext();
        if (next) {
          e.preventDefault();
          startEditing(next, { col, edge: "first" }, null, navigationSurface());
        } else {
          // No next LOADED block. In the journal feed, pull in the next day so
          // Down-arrow keeps going past the loaded window (previously only a
          // mouse-wheel scroll grew the feed). Non-feed pages resolve to null → a
          // harmless no-op. Async: flush first, then step into the new day.
          if (!outlineScope) {
            e.preventDefault();
            commit(raw);
            void navigateJournalDown({
              id: props.id, col, surface: navigationSurface(), editor: ref,
              current: origin => editorMounted && ref === origin && docNode(props.id) === shownNode,
            });
          }
        }
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      const revealed = codeView.revealRaw(raw, commit);
      if (revealed !== null) {
        ref.value = revealed; ref.setSelectionRange(revealed.length, revealed.length);
        ref.focus(); autosize();
      } else selectBlock(position().id, position().scope); // exit raw editing into block selection
    }
  };

  const onBlur = (e: FocusEvent) => {
    plainPaste.dispose();
    queueMicrotask(endCompositionHold); // an IME cancelled by blur may send no compositionend (L9-18); after the commits below
    // The palette must retain the source occurrence until its contextual
    // Select embedding block action runs. Blur still commits the draft.
    if (embedNavExit && (e.relatedTarget as HTMLElement | null)?.closest?.(".switcher")) {
      commit(ref.value);
      windowFocus.remember();
      return;
    }
    unregisterFocusedEditor();
    if (sheetCanceling) return;
    // The engine's blur on a textarea removed by a page replacement: not the
    // user leaving the block. The carried editor (if any) owns editing now.
    if (replacedUnderEditor()) return;
    // A block-move reorder blurs us momentarily — stay in edit mode (the move
    // handler refocuses and restores the caret). Commit as-is, don't normalize.
    if (isBlockMoving()) {
      commit(ref.value);
      return;
    }
    // Find and the date picker temporarily own focus; keep this editor and its
    // caret mounted so dismissal can return to the same editing transaction.
    if ((e.relatedTarget as HTMLElement | null)?.closest?.(".date-picker") || inPageFindPreservesEditorBlur()) {
      commit(ref.value);
      windowFocus.remember();
      return;
    }
    // Android can blur the WebView editor before document.hasFocus() reflects
    // that the external camera/file-picker activity covered the app. Still the
    // same edit transaction: keep its identity and caret until durable import
    // and insertion finish (GH #493/#622), including a delayed return blur.
    // Graph/block changes stay guarded by assetEditorIsCurrent after the await.
    if (assets.holdsBlur()) {
      commit(ref.value);
      windowFocus.remember();
      return;
    }
    // The editor's own window lost focus (switched to another app or window): stay
    // in edit mode and remember the caret so windowFocus can resume exactly here.
    // Commit as-is — we're still editing, not exiting. If the window that gains
    // focus next is another Tine window, windowFocus ends the edit there.
    if (!documentHasFocus(ref)) {
      commit(ref.value);
      windowFocus.remember();
      return;
    }
    const target = e.relatedTarget;
    const sameBlockEditor = isElementTag(target, "textarea")
      && target.classList.contains("block-editor")
      && target.closest("[data-block-id]")?.getAttribute("data-block-id") === props.id;
    exitEditOnBlur(sameBlockEditor ? target : undefined);
  };

  // A real exit (clicking elsewhere, Escape, Enter→new block, another Tine window
  // taking focus): move any SCHEDULED/DEADLINE planning line to its canonical
  // position (OG layout) as we commit — type-anywhere-while-editing,
  // normalize-on-exit (M1c). The editor is closing, so there is no caret to preserve.
  const exitEditOnBlur = (handoff?: HTMLTextAreaElement) => {
    const intentIsCurrent = captureEditIntent();
    const calcExit = isCalc();
    commitBlockEditorExit({ id: props.id, text: ref.value, format: pageFmt(), literal: calcExit || !!codeShown(), handoff: !!handoff,
      commit: text => commit(text, calcExit ? { calc: true } : undefined) });
    const endIfCurrent = () => {
      // Blur precedes the destination's focus event. Let it record the active
      // surface before deciding whether this occurrence still owns the exit.
      if (!intentIsCurrent() || editingId() !== props.id) return;
      if (handoff?.isConnected && documentOf(handoff).activeElement === handoff) return;
      if (handoff) finishPageHeaderEdit(props.id);
      endEdit("blur");
    };
    if (handoff) queueMicrotask(endIfCurrent);
    else endIfCurrent();
  };

  const windowFocus = editorWindowFocus({ id: props.id, element: () => ref,
    selection: () => ({ start: ref.selectionStart, end: ref.selectionEnd }),
    focus: saved => {
      ref.focus();
      resizeNow();
      if (saved) {
        const end = Math.min(saved.end, ref.value.length);
        ref.setSelectionRange(Math.min(saved.start, end), end);
      }
    },
    exit: () => exitEditOnBlur(),
  });

  // Paste copied files/images as graph assets. Native file lists use path-based
  // imports; browser-only file payloads use a bounded byte fallback. Ordinary
  // text keeps the existing structural/outline/link behavior below.
  const onPaste = (e: ClipboardEvent) => {
    // Consume the modifier latch on EVERY paste, including file/image pastes.
    // Otherwise an intercepted shortcut could affect a later context-menu paste.
    const rawPaste = plainPaste.consume();
    const text = e.clipboardData?.getData("text/plain") ?? "";
    if (rawPaste) {
      e.preventDefault();
      // Empty text explicitly claims the gesture but leaves the block untouched;
      // never fall through to an HTML/file flavor.
      if (!text) return;
      // OG 6e7afa8eb src/main/frontend/handler/paste.cljs:262-271 deletes the
      // selection and inserts exactly the clipboard text, bypassing every
      // formatted/file/block branch.
      pasteLiteralText(text);
      return;
    }
    const html = e.clipboardData?.getData("text/html") ?? "";
    // File managers commonly include path text alongside the real file-list
    // clipboard flavor. Claim the paste synchronously so those paths never land
    // as text; actual paths are accepted only from the native clipboard API.
    const { eventFiles, ...flavors } = clipboardEventFlavors(e.clipboardData, text);
    if (flavors.hasFiles) {
      e.preventDefault();
      const [value, from, to] = [ref.value, ref.selectionStart, ref.selectionEnd];
      void pasteClipboardFiles(eventFiles, flavors.canFallbackToText ? () => {
        if (ref.value === value && ref.selectionStart === from && ref.selectionEnd === to) pasteTextClipboard(e, flavors.text, html, true);
      } : undefined);
      return;
    }
    pasteTextClipboard(e, flavors.text, html, flavors.text !== text);
  };

  const pasteTextClipboard = (e: ClipboardEvent, text: string, html: string, insertPlain = false) => {
    const start = ref.selectionStart;
    const syntaxSensitive = sheetCell && !gridNestedBlock(props.id, sheetCell) || isCalc() || codeShown() !== null || caretInFence(ref.value, start, pageFmt())
      || caretOnOpeningFence(ref.value, start, pageFmt()) || caretInDisplayMath(ref.value, start, pageFmt());
    const slot = peekClipboardPayload();
    if (!syntaxSensitive) {
      if (slot && text !== "" && normalizeClipboardText(text) === normalizeClipboardText(slot.text)) {
        e.preventDefault();
        // Association is intentionally text-only and can replay the user's last
        // private block copy when a foreign clipboard happens to contain equal
        // normalized text. Identity remains separately one-shot and validated.
        void readOwned(graphOwner(() => editorMounted), pasteClipboardPayload(props.id, slot))
          .then((result) => {
            if (result.kind === "stale") return; const lastId = result.value;
            if (lastId && docNode(lastId)) runGridOutline(sheetCell, () => startEditing(lastId, docNode(lastId).raw.length));
          })
          .catch((error) => reportUiFailure("clipboard-association", error));
        return;
      }
    }
    // A non-empty observed replacement makes stale private data unusable even
    // if a later external clipboard happens to restore the old text. Matching
    // text in a syntax-sensitive surface is only a bypass, not a replacement.
    if (slot && text !== "" && normalizeClipboardText(text) !== normalizeClipboardText(slot.text)) clearClipboardPayload();
    // A structural sheet copy (multiple grid cells) pasted into a block editor
    // rebuilds an actual subgrid nested here, rather than dumping the flat TSV
    // text (Martin's nit). Only fires when the clipboard is exactly our own
    // sheet copy; anything else falls through to normal paste.
    const sheetGridNode = structuralSheetPasteNode(text);
    if (sheetGridNode) {
      e.preventDefault();
      insertPastedOutline([sheetGridNode], "sheet-outline-paste", true);
      return;
    }
    // Preserve explicit clipboard HTML structure on ordinary editor surfaces.
    const htmlPaste = syntaxSensitive ? null : structuredHtmlPaste(html, text, pageFmt());
    if (htmlPaste) {
      e.preventDefault();
      if (htmlPaste.inlineText !== null) pasteLiteralText(htmlPaste.inlineText);
      else insertPastedOutline(htmlPaste.outline, "structured-paste");
      return;
    }
    if (text.includes("\n")) {
      e.preventDefault();
      if (text.length > OUTLINE_MAX_SOURCE_CHARS) { pushToast(msg("editor-block-pasted-text-is-too-large"), "error"); return; }
      if (syntaxSensitive) {
        pasteLiteralText(text);
        return;
      }
      // One outline-module answer (editor/outline.ts): OG's bullet/heading test and paragraph split, with
      // literal source (fences, code) left whole. No blocks means the text is inserted as typed.
      const nodes = pastedPlainBlocks(text, pageFmt());
      if (!nodes) {
        pasteLiteralText(text);
        return;
      }
      if (!nodes.length) return;
      insertPastedOutline(nodes, "outline-paste");
      return;
    }
    // A bare URL pasted over a non-empty selection wraps the selection as a
    // link instead of replacing it (#23; logseq-copy-url-style QoL). Format
    // aware: md `[sel](url)` vs org `[[url][sel]]`. Skip in calc/code fences,
    // and when the selection is itself a URL (a normal replace is wanted then).
    {
      const start = ref.selectionStart;
      const end = ref.selectionEnd;
      const url = text.trim();
      if (
        start !== end &&
        isPasteableUrl(url) &&
        !isPasteableUrl(ref.value.slice(start, end)) &&
        !isCalc() &&
        codeShown() === null &&
        !caretInFence(ref.value, start, pageFmt()) &&
        !caretOnOpeningFence(ref.value, start, pageFmt())
      ) {
        e.preventDefault();
        applyEdit(wrapLink(ref.value, start, end, url, pageFmt()));
        queueMicrotask(updateSel);
        return;
      }
    }
    // OG 6e7afa8eb src/main/frontend/handler/paste.cljs:49-57,164-166 wraps
    // recognized bare video URLs only after selected-URL handling has declined.
    const videoMacro = autoEmbedVideoLinks() ? videoPasteMacro(text) : null;
    if (videoMacro && ref.selectionStart === ref.selectionEnd) {
      e.preventDefault();
      const start = ref.selectionStart;
      applyEdit({
        text: ref.value.slice(0, start) + videoMacro + ref.value.slice(ref.selectionEnd),
        start: start + videoMacro.length,
        end: start + videoMacro.length,
      });
      queueMicrotask(updateSel);
      return;
    }
    // URI-only payloads and asynchronous recovery have no browser text default.
    if (insertPlain && text) {
      e.preventDefault();
      pasteLiteralText(text);
      return;
    }
    // File/image event items were handled above. WebKitGTK can expose image
    // bytes without a file flavor, so retain the native image-reader fallback.
    // Single-line plain text still uses the browser's synchronous paste default.
    assets.pasteImage(e);
  };
  function insertPastedOutline(nodes: OutlineNode[], tag: string, asChildren = false) {
    const current = docNode(props.id);
    if (!current) return;
    if (!outlineFits(props.id, nodes, asChildren ? 1 : 0)) { pushToast(msg("editor-block-pasted-outline-is-too-deep"), "error"); return; }
    const rawAtPaste = ref.value;
    const pageAtPaste = current.page;
    const insert = (prepared: OutlineNode[] | null) => {
      if (!prepared?.length || !ref.isConnected || ref.value !== rawAtPaste || docNode(props.id)?.page !== pageAtPaste) return;
      // One gesture (og-enter-family-1109): the text as typed and the pasted outline are written together,
      // once the kernel has answered; a refusal or a parser failure writes neither.
      const lastId = insertEditorOutline(props.id, prepared, stage(rawAtPaste), tag, { children: asChildren, hide: hideFn(),
        tooDeep: msg("editor-block-pasted-outline-is-too-deep"), refused: msg("editor-block-pasted-outline-was-refused") });
      if (!asChildren && lastId && docNode(lastId)) runGridOutline(sheetCell, () => startEditing(lastId, docNode(lastId).raw.length));
    };
    const prepared = sanitizeOutlineIdsForPaste(props.id, nodes);
    if (prepared instanceof Promise) void prepared.then(insert);
    else insert(prepared);
  }
  return (
    <div class="editor-wrap" dir={blockDirAttr(docNode(props.id)?.raw ?? "", pageFmt())} classList={{ "calc-wrap": isCalc(), "code-wrapping": codeEditing() && codeWrapping() }}>
      <Show when={mixedCodeContext()?.before}>{before => <div class="block-content mixed-code-before" onMouseDown={event => editMixedContext(event, "before")}><BlockRenderBoundary raw={before()}><AstBody raw={before()} format={pageFmt()} headingLevel={effectiveHeadingLevel(propertySession.facets(node().raw, pageFmt()), depthOf(props.id))} /></BlockRenderBoundary></div>}</Show>
      <div class="code-editor-region" dir={codeEditing() || isCalc() ? "ltr" : undefined} classList={{ "mixed-code-region": !!mixedCodeContext() }}>
      <Show when={codeEditing()}>
        <span class="code-language">{(codeShown() ?? codeFenceOnly(editorValue(), pageFmt()))?.lang.toLowerCase()}</span>
        <Show when={codeShown() ?? codeBodyProjection(editorValue(), pageFmt())}>
          {code => <CopyButton text={code().body} title={translate("render-copy-code")} class="code-copy" preserveFocus />}
        </Show>
        <LineGutter lines={(codeShown()?.body ?? editorValue()).split("\n")} code />
      </Show>
      <Show when={isCalc()}>
        <LineGutter lines={calcRows().map(row => row.input)} />
      </Show>
      <textarea
        ref={ref}
        class="block-editor"
        classList={{ [`h${editorHeadingLevel()}`]: editorHeadingLevel() != null, "code-edit": codeEditing() }}
        wrap={codeEditing() && !codeWrapping() ? "off" : "soft"}
        spellcheck={spellcheckEnabled()}
        // A frozen page (a delete, transfer, rename or graph switch) takes
        // no input at all, so nothing typed is shown and then dropped (Q-TS2).
        readOnly={!blockWritable(props.id)}
        value={textareaValue()}
        placeholder={cap?.bulletHint?.() || PRIMARY_SELECTION_GUARD_PLACEHOLDER}
        onInput={onInput}
        onCompositionStart={beginComposition}
        onCompositionEnd={onCompositionEnd}
        onKeyDown={onKeyDown}
        onKeyUp={(e) => {
          if (shortcutKey(e) === "v") clearPasteRaw();
          if (e.key.startsWith("Arrow")) updateSel();
        }}
        onFocus={() => {
          noteSurfaceFocused(surfaceKey);
          registerFocusedEditorBridge();
        }}
        onBlur={onBlur}
        onPaste={onPaste}
        onSelect={updateSel}
        onMouseUp={updateSel}
        rows={1}
      />
      </div>
      <Show when={mixedCodeContext()?.after}>{after => <div class="block-content mixed-code-after" onMouseDown={event => editMixedContext(event, "after")}><BlockRenderBoundary raw={after()}><AstBody raw={after()} format={pageFmt()} /></BlockRenderBoundary></div>}</Show>
      <Show when={isCalc()}>
        <div class="calc-results">
          <For each={calcRows()}>
            {(r) => (
              <div class="calc-out" classList={{ "calc-error": !!r.error }}>
                {r.output ?? ""}
                <Show when={r.output !== null && !r.error}>
                  <CopyButton text={r.output!} title={translate("editor-block-copy-result")} class="calc-copy" />
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
      <Show when={hasSel()}>
        <div ref={selToolbar.toolbar} class="sel-toolbar" classList={{ "sel-toolbar-mobile": touchPrimary() }} data-mobile-selection-toolbar={touchPrimary() ? "" : undefined} onMouseDown={(e) => e.preventDefault()}>
          <For each={essentialSelectionActions}>{(action) => (
            <button
              classList={{ "sel-action-page-link": action.id === "page-link" }}
              title={action.title}
              aria-label={action.title}
              data-selection-action={action.id}
              onClick={() => runSelectionAction(action)}
            >{selectionActionLabel(action)}</button>
          )}</For>
          <div class="sel-toolbar-secondary">
            <For each={secondarySelectionActions}>{(action) => (
              <button
                title={action.title}
                aria-label={action.title}
                data-selection-action={action.id}
                onClick={() => runSelectionAction(action)}
              >{selectionActionLabel(action)}</button>
            )}</For>
          </div>
          <button
            class="sel-toolbar-more"
            title={translate("editor-block-more-formatting")}
            aria-label={translate("editor-block-more-formatting")}
            aria-haspopup="menu"
            aria-expanded={selectionOverflowOpen()}
            onClick={() => setSelectionOverflowOpen((open) => !open)}
          >…</button>
          <Show when={!cap}>
            <button type="button" data-selection-action="comment" title={translate("comment-toolbar-shortcut")} aria-label={translate("comment-toolbar")}
              onClick={() => commentFromEditor(props.id, ref, commit, surfaceKey, sheetCell && editsLikeOutline(sheetCell) ? cellOwner(sheetCell) : null)}>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 11.5a8.5 8.5 0 0 1-8.5 8.5H4l-3 3V11.5a10 10 0 0 1 20 0Z" /></svg>
            </button>
          </Show>
          <Show when={selectionOverflowOpen()}>
            <div ref={selectionOverflowRef} class="sel-toolbar-overflow" role="menu" aria-label={translate("editor-block-more-formatting")}>
              <For each={secondarySelectionActions}>{(action) => (
                <button
                  role="menuitem"
                  title={action.title}
                  aria-label={action.title}
                  data-selection-action={action.id}
                  onClick={() => runSelectionAction(action)}
                >{selectionActionLabel(action)}</button>
              )}</For>
            </div>
          </Show>
        </div>
      </Show>
      <Show when={acVisible() && acRect()}>
        <EditorAutocomplete items={acItems()} index={acIndex()} style={acStyle()}
          listRef={(element) => { acListRef = element; queueMicrotask(updateAcRect); }} select={selectAc}
          blockState={acBlockState()} retry={() => void updateAutocomplete()} />
      </Show>
    </div>
  );
}

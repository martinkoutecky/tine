import { shortcutKey } from "../shortcutKey";
import { bindCollapseCommandSurface } from "../collapseCommandSurface";
import { msg, t as translate } from "../i18n";
import { codeWrapping } from "../codeDisplay";
import { stickyParents } from "../stickyParents";
import { StickyAncestor } from "./StickyAncestor";
import { AstBody } from "../render/body";
import { codeFences, caretInFence, caretOnOpeningFence, caretInDisplayMath } from "../editor/fences";
import { LineGutter } from "../render/LineGutter";
import { Show, Switch, Match, For, createMemo, createSignal, createContext, useContext, createUniqueId, createEffect, on, untrack, onMount, onCleanup, type JSX } from "solid-js";
import { autocompleteFacets, backend } from "../backend";
import { pinyinMatchingEnabled } from "../launcherRanking";
import { reportUiFailure } from "../uiFailure";
import { clearClipboardSlot, normalize, peekClipboardSlot, readClipboardText } from "../clipboard";
import {
  detectTrigger,
  applyCompletion,
  refCompletionEnd,
  withRefCompletionSpace,
  autoPairEdit,
  applyFullWidthRefReplace,
  applyFullWidthCloserThrough,
  pageInsert,
  tagInsert,
  orderAcItems,
  COMMANDS,
  advancedBlockInsertion,
  filterAdvancedBlockCommands,
  commandScore,
  codeLanguageItems,
  fuzzyScore, aliasOfLabel,
  propertyKeyFold,
  propertyValueKeyAfterBoundary,
  type Trigger,
} from "../editor/autocomplete";
import { navigationName } from "../pageIndex";
import { pluginManager } from "../plugins/manager";
import { bindPluginBlockSnapshot, isPluginGraphOwnerCurrent } from "../plugins/ownership";
import { autoPairInsertOnInput, wrapSelectionEdit, doubleRefKind, backspacePairEdit, SELECTION_WRAP } from "../editor/autopair";
import { typoTypeReplace } from "../render/typography";
import { rangeInLiteral } from "../editor/inlineLiteral";
import { linkAutocompletePolicy } from "../editor/linkDefault";
import { spellcheckEnabled } from "../spellcheckSettings";
import { restoreMovedSelection, keepCaretThroughRewrite, syncTextareaToStore } from "../editor/restoreMovedSelection";
import { navigateJournalDown } from "../editor/journalNavigation";
import { focusMountedEditor } from "../editor/surfaceFocus";
import { spaceAfterRefCompletion } from "../refCompletionSettings";
import { pageByName, blockPageReadOnly, blockWritable, setRaw, makeOwnNumberedList, removeOwnNumberedList, stopOwnNumberedListOnEmptyEnter, splitBlock, indentBlock, outdentBlock, mergeWithPrev, mergeWithNext, setCollapsed, prevVisible, nextVisible, beginPageHeaderEdit, finishPageHeaderEdit, insertOutlineAfter, replaceEmptyBlockWithOutline, insertOutlineChildren, outlineFits, pasteClipboardPayload, sanitizeOutlineIdsForPaste, deleteBlock, moveBlockFeed, moveItem, selectBlock, selectBlockSubtree, moveSelection, isSelected, persistBlockRefTarget, isBlockMoving, withBlockMoving, orderedListMarker, withUndoUnit, blockIsGridView, depthOf, setHeading, blockExternalId, pinPageWhileDrafting, blockHasFoldableBody, blockHasPageEmbed, blockHasQueryBody, storedFold, type OutlineScope, node as docNode } from "../document";
import { openDurableBlock, pasteAsLinkEdit } from "../blockRefActions";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import {
  editingId,
  editingOwner,
  captureEditIntent,
  editingSurface,
  endEdit,
  noteSurfaceFocused,
  registerHistoryEditorTarget,
  startEditing,
  takeCaretFor,
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
  setPriority,
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
import { isMobilePlatform } from "../nativeChrome";
import { touchPrimary } from "../inputCapability";
import { openJournalDatePicker, runJournalSlash } from "../journalSlash";
import { calcSource, serializeCalcExitCommit, evalCalc } from "../editor/calc";
import { youtubeTimestampMacroFor, insertYoutubeTimestamp } from "./youtubeTimestampEdit";
import { Rendered, detectMacro } from "./Rendered";
import { SeenRemoved, useSeenRow } from "../seen/SeenHeader";
import { CollapseAllBorder, NO_THREAD_LINES, rowDecorationClasses, rowDecorationStyle, threadLineSettings, type ThreadLineDecoration } from "./block/rowChrome";
import { dbg } from "../debug";
import { workflow, zoomInto, openContextMenu, openDatePicker, setQueryBuilderAutoOpen, openPageProps, autoPairing, typographyMode, blockReferencesRequest, documentMode, docModeEnterForNewBlock, searchRemoveAccents } from "../ui";
import { ResultWriteTargetContext, liveWriteTarget } from "./queryResultIdentity";
import { toggleViewFold, viewFoldSurface } from "./block/readOnlyFold";
import { pushToast } from "../toasts";
import { copyBlockLink } from "./blockLinkCopy";
import { editorAssets } from "../editor/editorAssets";
import { captureBinding, bindingCurrent } from "../binding";
import { graphOwner, latestOwner, ownedWhen, readOwned } from "../owned";
import { EditorAutocomplete } from "./EditorAutocomplete";
import { getTemplates, templateToOutline, type AcItem } from "../editor/completionItems";
import { parserReady } from "../render/parse";
import { BlockReferences } from "./BlockReferences";
import { editorCommandFor, isPermittedTabGesture, isTabLikeEvent } from "../keybindings";
import { cycleMarkerSmart } from "../editor/repeat";
import { setMarker } from "../editor/marker";
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
import { editorCommitRecovery } from "../editor/commitRecovery";
import { QUERY_MACRO_SCAFFOLD } from "../editor/queryMacro";
import { normalizePlanning } from "../editor/planning";
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
import { blockDtoExternalId } from "../blockIdentity";
import { SheetContainer } from "./SheetContainer";
import { ownsBlockContextMenu, shouldOpenBlockContextMenu } from "../contextMenuPolicy";
import { followBlockSwipe } from "./blockSwipeWiring";
import { beginDrag, beginEditGesture, bulletDragMoved, dragId, dropInd, renderedClickOffset } from "./blockGestures";
import { noticeMoveOutOfView } from "../moveNotice";
import { captureEditorScrollAnchor } from "../editor/scrollAnchor";
import { PRIMARY_SELECTION_GUARD_PLACEHOLDER } from "../textareaPrimarySelection";
import { blockFirstLine, formatForBlockId, isBlockEmbedMacro, listLineAt, nearestScrollableY, resizeBlockEditor, timeStamp } from "./blockParts";
import { commentFromEditor, replyOnEnter } from "../commentActions";
import { AuthorChip, CommentActions, CommentComposer, CommentQuoteHeader, MarginMarker, QuoteHighlightProvider } from "./CommentParts";
import { commentFacts, marginNavigation, marginOutline } from "./blockComments";
import { BlockRefCountBadge } from "./BlockRefCountBadge";
import { documentHasFocus, documentOf, isElementTag, listen, newResizeObserver, nextFrame, onEachWindow, requestFrame, viewportOf, windowOf } from "../windowRealm";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { cm6EditorEnabled } from "../editor/cm6Setting";
import { applySheetViewSlashAction } from "./block/sheetViewSlash";
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
export const EmbedNavExitContext = createContext<{ hostBlockId: string; firstRoot: () => string | undefined } | null>(null);
/** A plain bullet click opens the block here instead of zooming the pane (a
 *  hover preview opens it in a new tab, D-20261023-VQ). */
export const BulletOpenContext = createContext<((id: string) => void) | null>(null);
export interface CollapseSurfaceApi {
  collapsed: (id: string, stored: boolean) => boolean;
  toggle: (id: string, current: boolean) => void;
  setMany: (ids: readonly string[], collapsed: boolean) => void;
}
// Deliberate Tine divergence from OG Logseq: a secondary/transcluded rendering
// never mutates its source. A block embed follows the source until its macro
// host records an explicit occurrence-owned collapse override; reference/query
// surfaces keep their local presentation state. Keep the surface contract
// explicit when changing collapse parity.
export const CollapseSurfaceContext = createContext<CollapseSurfaceApi | null>(null);

/** Render and edit one document block through the document door. Work scales
 * with its visible descendants; a failed structured paste shows fixed text. */
export function Block(props: { id: string; hideRefCount?: boolean; forceExpanded?: boolean }): JSX.Element {
  // Share one node read across this block's derivations.
  const node = createMemo(() => docNode(props.id));
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
  const editing = () => {
    if (editingId() !== props.id) return false;
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
  bindCollapseCommandSurface(props.id, collapseSurface, outlineScope, editing);
  const collapsed = () => viewCollapseSurface()?.collapsed(props.id, storedFolded()) ?? storedFolded();
  const fmt = createMemo(() => pageByName(node().page)?.format ?? "md");
  const blockFacets = createMemo<Facets>(() => {
    const n = node();
    return n ? propertySession.facets(n.raw, fmt()) : EMPTY_FACETS;
  });
  // Margin dialogue (vision §3.7): comments leave the main outline for the margin.
  const { outlineChildren, marginComments } = marginOutline(node, surfaceKey, fmt);
  const hasChildren = () => outlineChildren().length > 0;
  const hasFoldableBody = () => hasChildren() || blockHasPageEmbed(props.id) || blockHasQueryBody(props.id); // a query host folds its results (OG; GH #619)
  // The children-source sheet this block owns (a query block's table/board is the macro's).
  const sheet = createMemo(() => childrenSheetConfig(blockFacets().properties, node().raw));
  // `thread-lines` only decorates the ordinary outline container below this row,
  // so a leaf (most blocks of a large flat page) does not subscribe to plugin
  // installation/settings at all. Collapsed parents stay eligible so their
  // decoration is already current when they expand (master ce9a796fb). A plain
  // function, not a memo: a memo would allocate a reactive node per block to
  // answer "false" thousands of times.
  const threadLineDecoration = (): ThreadLineDecoration => {
    if (!(hasChildren() && sheet().view === null)) return NO_THREAD_LINES;
    return threadLineSettings(pluginManager.hasDeclarativeDecoration("thread-lines"),
      key => pluginManager.declarativeDecorationSetting("thread-lines", key));
  };
  // Heading level of THIS block's first line, so the bullet column can match the
  // (taller) heading line box and the bullet stays centered on it. Shared with
  // `Rendered`, so the `depthOf` parent walk happens once per block, not twice.
  const headingLevel = createMemo(() => effectiveHeadingLevel(blockFacets(), depthOf(props.id)));
  // Editor-only derivations, read solely while THIS block is being edited: lazy,
  // so a page load never runs `splitProps` over every block's raw text.
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
  // GH #726: OG block.cljs block-embed/page-embed keep the source outline
  // inside the host's content. Both host and source retain their own handles;
  // every menu and drag acts on the block whose bullet was pressed.
  const macro = createMemo(() => detectMacro(node().raw, fmt())); // shared with `Rendered`
  const blockEmbedHost = createMemo(() => isBlockEmbedMacro(macro()));
  const { commentSelector, author, showsAuthor, bulletless, commentClasses } = commentFacts(node, () => blockFacets().properties, fmt);
  const stickyParent = () => stickyParents() && !bulletless() && hasChildren() && (props.forceExpanded || !collapsed()) && !macro(); // Thread rows have no bullet pins; the spacer would pull the body over its quote.
  let mainEl: HTMLDivElement | undefined;

  return (
    <div
      class="ls-block"
      {...(() => {
        const thread = threadLineDecoration();
        return {
          style: rowDecorationStyle(thread, thread.enabled && thread.levels ? depthOf(props.id) : 0),
          classList: {
            collapsed: collapsed(),
            "block-embed-host": blockEmbedHost(),
            ...commentClasses(),
            ...rowDecorationClasses(thread),
          },
        };
      })()}
      data-block-id={props.id}
      data-sticky-parent={stickyParent() ? "" : undefined}
      data-block-ref={parserReady() ? blockExternalId(props.id, propertySession.identity(node().raw, fmt())) ?? props.id : undefined}
    >
      <Show when={stickyParent()}><StickyAncestor id={props.id} raw={node().raw} /></Show>
      <CommentQuoteHeader parentId={node().parent} selector={commentSelector()} controls={
        <Show when={bulletless()}>
          <span class="comment-first-line-controls">
            <Show when={showsAuthor()}><AuthorChip author={author()} handleOf={props.id} /></Show>
            <CommentActions id={props.id} />
          </span>
        </Show>
      } />
      <div
        class="block-main"
        ref={(el) => {
          mainEl = el;
          // GH #501: touch swipes on the row while a touch input exists, following
          // it if it appears or goes away (src/blockSwipe.ts).
          followBlockSwipe(el, { id: props.id, scope: outlineScope, editing, readOnly });
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
          openContextMenu(e.clientX, e.clientY, props.id);
        }}
      >
        <Show when={!bulletless()}>
        <div class="block-controls">
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
          <span
            class="bullet-container"
            classList={{ "bullet-closed": collapsed() && hasFoldableBody(), ordered: !!orderMarker() }}
            title={translate("editor-block-click-to-zoom-shift-click-sidebar-ctrl-cmd-click-or-middle-click-new-tab-alt-click-other-pane-drag-to-move")}
            onMouseDown={(e) => {
              // Shared link gesture contract (GH #207): suppress shift-range selection
              // and middle-button autoscroll / PRIMARY-paste the destinations replace.
              internalLinkMouseDown(e);
              if (e.button === 0 && !readOnly()) beginDrag(props.id, e);
            }}
            onClick={(e) => {
              e.stopPropagation();
              if (bulletDragMoved()) return; // was a drag, not a click
              // GH #456: the same one decision every internal link uses (GH #283).
              switch (internalLinkDest(e)) {
                case "sidebar": openDurableBlock(props.id, "sidebar"); break;
                case "background": openDurableBlock(props.id, "tab"); break;
                case "pane": openDurableBlock(props.id, "pane"); break;
                default: if (bulletOpen) bulletOpen(props.id); else zoomInto(props.id);
              }
            }}
            onAuxClick={(e) => {
              if (internalLinkAuxClick(e, () => openDurableBlock(props.id, "tab"))) e.stopPropagation();
            }}
          >
            <Show when={orderMarker()} fallback={<span class="bullet" />}>
              <span class="bullet-order">{orderMarker()}.</span>
            </Show>
          </span>
        </div>
        </Show>

        <div
          class="block-content-wrapper"
          classList={{ "read-only": readOnly() }}
          onMouseDown={(e) => {
            // Mousedown anywhere in the row (not on a link/chip) arms the same
            // click-or-drag gesture as block content, with an end-of-block caret
            // (the row padding has no text to map). Read-only org pages don't edit.
            if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return;
            if (!editing() && !readOnly() && !forbidsEditEntry(e))
              beginEditGesture(e, props.id, docNode(props.id).raw.length, instanceId, outlineScope);
          }}
        >
          <Show when={bulletless() && !commentSelector()}>
            <span class="comment-first-line-controls">
              <Show when={showsAuthor()}><AuthorChip author={author()} handleOf={props.id} /></Show>
              <CommentActions id={props.id} />
            </span>
          </Show>
          <Show
            when={editing()}
            fallback={
              <QuoteHighlightProvider parentId={props.id} format={fmt()} when={node().children.length > 0}>
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
                    <BlockRefCountBadge id={props.id} hidden={props.hideRefCount} open={showRefs()} toggle={() => setShowRefs((v) => !v)} />
                    <Show when={showsAuthor() && !bulletless()}>
                      <AuthorChip author={author()} handleOf={bulletless() ? props.id : undefined} />
                    </Show>
                  </>}
                />}
              </QuoteHighlightProvider>
            }
          >
            <Editor id={props.id} propertySession={propertySession} />
          </Show>
        </div>
        <MarginMarker parentId={props.id} comments={marginComments()} />
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
                <For {...blockListProps(outlineChildren)} />
              </div>
            </div>
          </Match>
        </Switch>
      </Show>
      <Show when={commentClasses()["comment-card"]}><CommentComposer id={props.id} surface={surfaceKey} /></Show>
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
]);


// Block editor for `props.id`. Inside quick capture, CaptureCtx repurposes
// Enter/Escape when autocomplete is closed to commit or dismiss the capture.

export function Editor(props: { id: string; propertySession?: ReturnType<typeof propertyEditorSession> }): JSX.Element {
  if (cm6EditorEnabled()) return <Cm6BlockEditor id={props.id} surface={useContext(SurfaceContext)} scope={useContext(OutlineScopeContext)} propertySession={props.propertySession} />;
  const propertySession = props.propertySession ?? propertyEditorSession();
  propertySession.begin();
  // Non-null only inside the quick-capture window (see CaptureCtx).
  const cap = useContext(CaptureCtx);
  const sheetCell = useContext(SheetCellContext);
  // Which surface (main pane / a specific sidebar item) this editor lives in —
  // drives edit-focus arbitration when the same block renders in several surfaces.
  const surfaceKey = useContext(SurfaceContext);
  const outlineScope = useContext(OutlineScopeContext);
  const embedNavExit = useContext(EmbedNavExitContext);
  // Inside query results, the editor's pickers write only if no result changed meanwhile (GH #433 review).
  const inResults = useContext(ResultWriteTargetContext);
  const resultTarget = (id: string) => inResults(id) ? liveWriteTarget(id) : undefined;
  const collapseSurface = useContext(CollapseSurfaceContext);
  // Ref/query arrow navigation stays in the rendered result surface (master
  // GH #341), while structural edits still target the source outline: a
  // split/merge destination need not remain a query or backlink result. Embeds
  // are true transclusions, so both navigation and structural destinations stay
  // there.
  const navigationSurface = () =>
    surfaceKey.startsWith("ref:") || surfaceKey.startsWith("embed:") ? surfaceKey : null;
  const editSurface = () => surfaceKey.startsWith("embed:") ? surfaceKey : null;
  // A navOnly display-list scope must never act as a merge/structural topology.
  const structuralScope = outlineScope?.navOnly ? null : outlineScope;
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
  let pluginSlashInvocation = 0;
  let editorMounted = true;
  onCleanup(() => {
    editorMounted = false;
    pluginSlashInvocation++;
  });
  const autocompleteLayerId = `block-completion-${createUniqueId()}`;
  const selectionOverflowLayerId = `block-selection-overflow-${createUniqueId()}`;
  // Caret/selection stashed when the *window* (not this block) loses focus, so
  // returning to Tine resumes editing exactly where you left off.
  let savedSel: { start: number; end: number } | null = null;
  const node = () => docNode(props.id);
  // The node this editor instance was built to show. A page replacement (an
  // external change, the host's answer to an Open) purges the page's nodes and
  // builds new ones, often under the same ids (runtime ids are structural), and
  // remounts the outline; `upsertPage` carries the editor to the new copy.
  // Chromium-family webviews then fire `blur` on this instance's removed,
  // focused textarea. Such an instance is stale: its text is the pre-reload
  // block, so writing it would silently revert the external change, and
  // ending the edit would close the carried editor (UI-CARRY-RELOAD-STALE-BLUR).
  const shownNode = node();
  const replacedUnderEditor = () => !editorMounted && (!node() || node() !== shownNode);
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
  // Parsed editor facts travel with the committed buffer; hidden bytes survive.
  const isAnnot = () => annotationInfo(propertySession.facets(node().raw, pageFmt()).properties) !== null;
  const hideFn = () => (isAnnot() ? HIDE_ALL : sheetCell ? SHEET_CELL_HIDE : BUILTIN_HIDE);
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
  let completionOriginalRaw: string | null = null;
  let completionLease: { pageName: string; blockId: string; originalRaw: string; session: string } | null = null;
  const releaseLease = (lease: NonNullable<typeof completionLease>) => {
    // Releasing an ephemeral resource must finish even after its editor dies.
    void readOwned(ownedWhen(() => true), backend().quickSwitch("", 0, lease, false))
      .catch(error => dbg(`completion baseline release: ${String(error)}`));
  };
  const releaseCompletionLease = () => {
    const lease = completionLease;
    completionLease = null;
    if (lease) releaseLease(lease);
  };
  onCleanup(releaseCompletionLease);
  let rawBeforeLastCommit = node().raw;
  const commitRecovery = editorCommitRecovery(() => ref, () => node() === shownNode, () => ref.value);
  const commit = (text: string, opts?: { timetracking?: boolean; calc?: boolean }) => {
    // A disposed editor whose block was replaced never writes (see shownNode).
    if (replacedUnderEditor()) return;
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
    if (!commitAsCalc && !codeShown() && text === editorValue()) { commitRecovery.applied(); return; }
    // For a code wrapper `text` is the payload body: re-attach the exact wrapper
    // bytes (GH #412/#413: the body-only projection is reversible).
    const visible = commitAsCalc ? serializeCalcExitCommit(text, editorValue()) : text;
    const next = commitRecovery.join(() => (!commitAsCalc ? codeWrapCommit(text) : null) ?? propertySession.join(visible, editorParts().hidden, node().raw, hideFn(), pageFmt()), text);
    if (next === node().raw) return;
    const setRawOpts = opts && "timetracking" in opts ? { timetracking: opts.timetracking } : undefined;
    // GH #515: capture once for the autosize frame, before live mirrors above react.
    if (pendingScrollAnchor === undefined) {
      pendingScrollAnchor = ref && documentOf(ref).activeElement === ref
        ? captureEditorScrollAnchor(ref, nearestScrollableY(ref)) : null;
    }
    autosize();
    rawBeforeLastCommit = node().raw;
    keepCaretThroughRewrite(ref, !commitAsCalc && !codeShown() && !compositionActive, () => setRaw(props.id, next, setRawOpts));
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

  const [ac, setAc] = createSignal<Trigger | null>(null);
  const [acItems, setAcItems] = createSignal<AcItem[]>([]);
  const [acBlockState, setAcBlockState] = createSignal<"pending" | "error" | "ready" | null>(null);
  const acVisible = () => !!ac() && (acItems().length > 0 || acBlockState() !== null);
  const [acIndex, setAcIndex] = createSignal(0);
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
  const openFenceLanguagePicker = (raw: string, fenceEnd: number) => {
    setAc({ kind: "code-language", query: "", start: fenceEnd, end: fenceEnd });
    setAcIndex(0);
    setAcItems(codeLanguageItems("").map((language) => ({
      get label() { return language.label; },
      sub: [language.id, ...language.aliases].join(" · "),
      insert: language.id,
      caret: language.id.length + 1,
    })));
    queueMicrotask(() => {
      ref.value = raw;
      ref.setSelectionRange(fenceEnd, fenceEnd);
      ref.focus();
      autosize();
    });
  };
  const [propertyValueKey, setPropertyValueKey] = createSignal<string | null>(null);
  let propertyFacets: [string, string[]][] = [];
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

  const closeAc = () => {
    releaseCompletionLease();
    completionOriginalRaw = null;
    setAc(null);
    setAcBlockState(null);
    setAcItems([]);
    setAcIndex(0);
    setPropertyValueKey(null);
  };
  const sameAcTrigger = (left: Trigger | null, right: Trigger): boolean =>
    left !== null &&
    left.kind === right.kind &&
    left.query === right.query &&
    left.start === right.start &&
    left.end === right.end &&
    left.property === right.property &&
    (left.propertyValues ?? []).join("\0") === (right.propertyValues ?? []).join("\0");
  const detectEditorTrigger = (value = ref.value, caret = ref.selectionStart): Trigger | null =>
    isCalc() || isAnnot() || !!sheetCell
      ? null
      : detectTrigger(value, caret, propertyValueKey(), pageFmt());
  const propertyValueItems = (key: string, query: string, used: readonly string[] = []): AcItem[] => {
    const values = propertyFacets.find(([candidate]) => candidate === key)?.[1] ?? [];
    const q = query.trim();
    const excluded = new Set(used.map((value) => value.toLocaleLowerCase()));
    const ranked = values
      .map((value, index) => ({ value, index, score: q ? fuzzyScore(q, value) : 1 }))
      .filter(({ value, score }) => score > 0 && !excluded.has(value.toLocaleLowerCase()))
      .sort((left, right) => right.score - left.score || left.index - right.index)
      .slice(0, 100)
      .map(({ value }) => ({ label: value, propertyValue: value }));
    if (q && !values.some((value) => value.toLowerCase() === q.toLowerCase())) {
      ranked.push({ get label() { return translate("editor-block-create", { name: String(q) }); }, propertyValue: q });
    }
    return ranked;
  };
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

  const autocompleteScope = {};
  const updateAutocomplete = async () => {
    if (!editorMounted || !node()) return;
    const t = detectEditorTrigger();
    if (!t) {
      closeAc();
      return;
    }
    setAc(t);
    setAcBlockState(null);
    setAcIndex(0);
    const requestOwner = latestOwner(autocompleteScope, "suggestions", graphOwner(() => editorMounted && sameAcTrigger(ac(), t)));
    if (t.kind === "property-name") {
      let facets: [string, string[]][];
      try {
        facets = await autocompleteFacets();
      } catch (error) {
        // Completion is an optional aid: a transient facet-query failure must
        // never reject the editor input event or pile up global error toasts.
        dbg(`property-autocomplete: ${String(error)}`);
        if (sameAcTrigger(ac(), t)) setAcItems([]);
        return;
      }
      const cur = ac();
      if (!sameAcTrigger(cur, t)) return;
      propertyFacets = facets;
      const q = t.query.trim();
      const ranked = facets
        .map(([key], index) => ({ key, index, score: q ? fuzzyScore(q, key) : 1 }))
        .filter(({ score }) => score > 0)
        .sort((left, right) => right.score - left.score || left.index - right.index)
        .slice(0, 100)
        .map(({ key }) => ({ label: key, propertyName: key }));
      const created = propertyKeyFold(t.query);
      if (created && !facets.some(([key]) => key === created)) {
        ranked.unshift({ get label() { return translate("editor-block-create", { name: String(created) }); }, propertyName: created });
      }
      setAcItems(ranked);
      return;
    }
    if (t.kind === "property-value") {
      setAcItems(propertyValueItems(t.property!, t.query, t.propertyValues));
      return;
    }
    if (t.kind === "code-language") {
      setAcItems(codeLanguageItems(t.query).map((language) => ({
        get label() { return language.label; },
        sub: [language.id, ...language.aliases].join(" · "),
        insert: language.id,
        // If a completed fence scaffold already follows, land on its code line;
        // a hand-typed one-line fence stays at the end and Enter behaves normally.
        caret: language.id.length + (ref.value[t.end] === "\n" ? 1 : 0),
      })));
      return;
    }
    if (t.kind === "advanced-command") {
      const fmt = formatForBlockId(props.id);
      setAcItems(filterAdvancedBlockCommands(t.query).map((command) => {
        const insertion = advancedBlockInsertion(command, fmt);
        return { get label() { return command.label; }, insert: insertion.insert, caret: insertion.caret };
      }));
      return;
    }
    if (t.kind === "command") {
      const q = t.query;
      const tmpls = await getTemplates();
      const cur = ac();
      if (!sameAcTrigger(cur, t)) return; // trigger changed while awaiting
      // Commands AND templates in one fuzzy-ranked list, so a strong template
      // match can outrank a weak command (and vice-versa). Empty query (bare `/`)
      // lists all commands in defined order, no templates. `idx` preserves the
      // defined order — commands before templates — as the stable tiebreaker.
      const showAllTemplates = !!q && "template".startsWith(q.toLowerCase()); // /t…/template lists them all
      const scored: { item: AcItem; s: number; idx: number }[] = [];
      COMMANDS.forEach((c) => {
        // /drawio launches an external editor — desktop only (GH #38).
        if (c.action === "drawio" && isMobilePlatform) return;
        const s = q ? commandScore(q, c) : 1;
        if (s > 0)
          scored.push({ item: { get label() { return c.label; }, insert: c.insert, caret: c.caret, action: c.action, taskMarker: c.taskMarker }, s, idx: q ? c.matchTieOrder : c.bareOrder });
      });
      pluginManager.slashCommands().forEach(({ pluginId, contribution }, i) => {
        const s = q ? fuzzyScore(q, contribution.title) : 1;
        if (s > 0) {
          scored.push({
            item: {
              label: contribution.title,
              sub: "Plugin",
              plugin: { pluginId, contributionId: contribution.id, insertText: contribution.insertText },
            },
            s,
            idx: COMMANDS.length + i,
          });
        }
      });
      if (q) {
        tmpls.forEach((tp, j) => {
          const s = showAllTemplates ? 1 : fuzzyScore(q, tp.name);
          if (s > 0)
            scored.push({
              item: { get label() { return translate("editor-block-template", { name: String(tp.name) }); }, templateNodes: tp.blocks },
              s,
              idx: COMMANDS.length + pluginManager.slashCommands().length + j,
            });
        });
      }
      scored.sort((a, b) => b.s - a.s || a.idx - b.idx);
      setAcItems(scored.map((x) => x.item));
      return;
    }
    if (t.kind === "block") {
      // `((` searches blocks by page; bare `((` stays hidden. Selection inserts
      // the target's durable external ID (see selectAc).
      setAcItems([]);
      setAcBlockState("pending");
      let result: import("../owned").Owned<import("../types").RefGroup[]>;
      try {
        result = await readOwned(requestOwner, backend().search(t.query, 20, "block-picker"));
      } catch (error) {
        if (requestOwner()) {
          dbg(`block-picker: ${String(error)}`);
          setAcBlockState("error");
        }
        return;
      }
      if (result.kind === "stale") return;
      setAcBlockState("ready");
      const items: AcItem[] = [];
      for (const g of result.value) {
        for (const b of g.blocks) {
          items.push({
            label: blockFirstLine(b.raw, pageByName(g.page)?.format === "org" ? "org" : "md") || g.page,
            sub: g.page,
            blockRef: { uuid: b.id, externalId: blockDtoExternalId(b), page: g.page, kind: g.kind },
          });
        }
      }
      setAcItems(items);
      return;
    }
    // OG leaves blank page/tag search active but renders no rows and does not
    // ask the backend for its legacy all-pages result.
    const q = t.query.trim();
    if (!q) {
      setAcItems([]);
      return;
    }
    completionOriginalRaw ??= node().raw;
    const editor = { pageName: node().page, blockId: props.id, originalRaw: completionOriginalRaw };
    if (!cap && pinyinMatchingEnabled() && !completionLease) {
      completionLease = { ...editor, session: crypto.randomUUID() };
    }
    const lease = completionLease;
    // Closing can overtake a blocking native query. Release again after its
    // reply (including failure) so it cannot retain obsolete baseline keys.
    let result: import("../owned").Owned<import("../types").PageEntry[]>;
    try {
      result = await readOwned(requestOwner, cap ? cap.quickSwitch(t.query, 100) : backend().quickSwitch(t.query, 100, lease ?? editor, pinyinMatchingEnabled()));
    } finally {
      if (lease && lease !== completionLease) releaseLease(lease);
    }
    if (result.kind === "stale") return;
    const pageItem = (name: string): AcItem =>
      t.kind === "page"
        ? { label: name, insert: pageInsert(name), get sub() { return aliasOfLabel(name, navigationName(name)); } }
        : { label: `#${name}`, insert: tagInsert(name), get sub() { return aliasOfLabel(name, navigationName(name)); } }; // tag context reads "#name"
    const createItem: AcItem =
      t.kind === "page"
        ? { get label() { return translate("editor-block-create", { name: String(q) }); }, insert: pageInsert(q) }
        : { get label() { return translate("editor-block-create-tag", { name: String(q) }); }, insert: tagInsert(q) };
    setAcItems(orderAcItems(
      result.value.map((page) => ({ name: page.name, matchClass: page.match_class, item: pageItem(page.name) })),
      { name: q, item: createItem },
      { query: q, policy: linkAutocompletePolicy(), removeAccents: searchRemoveAccents() },
    ));
  };

  createEffect(on(pinyinMatchingEnabled, () => {
    const trigger = untrack(ac);
    if (trigger?.kind === "page" || trigger?.kind === "tag") {
      if (!pinyinMatchingEnabled()) releaseCompletionLease();
      void updateAutocomplete();
    }
  }, { defer: true }));

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
    if (!editorMounted || !ref.isConnected || !node()) return;
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
  const closeAutocompleteIfOutside = () => {
    const open = ac();
    if (!open || (open.kind !== "page" && open.kind !== "tag" && open.kind !== "block")) return;
    const here = detectEditorTrigger();
    if (here && here.kind === open.kind && here.start === open.start) return;
    clearTimeout(acTimer);
    closeAc();
  };
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

  // Insert `text` in place of the active trigger and restore the caret. If the
  // completion ends with a closing pair (`]]`/`))`/`}}`) and the same pair
  // sits at or later on this line after the caret (e.g. from a `[[ ]]` autopair
  // or editing inside an existing ref), swallow it to avoid duplicate closers.
  const replaceTrigger = (text: string, caret?: number) => {
    const t = ac();
    if (!t) return;
    const end = refCompletionEnd(ref.value, t.end, text);
    const r = applyCompletion(ref.value, t.start, end, text, caret);
    // GH #35: after a page/block-ref completion whose caret lands at the natural end
    // (right after the closing `]]`/`))`), optionally insert a trailing space so the
    // next word flows on without reaching past the brackets. Tine default ON; toggle
    // off in Settings → Editor to match Logseq. Skipped when an explicit caret was
    // requested. commit() trims a block-final trailing space, so it never persists.
    const spaced =
      caret === undefined
        ? withRefCompletionSpace(r.raw, r.caret, text, spaceAfterRefCompletion())
        : r;
    // Let a calculator completion enter calc mode in the mounted editor (GH #57).
    const enteredCalc = !editingCalc() ? calcSource(spaced.raw) : null;
    // A completion that lands a COMPLETE code wrapper (the language pick on the
    // opener line) swaps to the body-only view: map the caret from raw into body
    // space, the same transition as calc above.
    const codeWrap = enteredCalc === null ? codeBodyProjection(spaced.raw, pageFmt()) : null;
    commit(spaced.raw);
    if (enteredCalc !== null) setEditingCalc(true);
    closeAc();
    queueMicrotask(() => {
      const shown = enteredCalc ?? codeWrap?.body ?? spaced.raw;
      const openingEnd =
        enteredCalc !== null ? spaced.raw.indexOf("\n") + 1 : codeWrap ? codeWrap.open.length : 0;
      const shownCaret = enteredCalc !== null || codeWrap
        ? Math.max(0, Math.min(shown.length, spaced.caret - openingEnd))
        : spaced.caret;
      ref.value = shown;
      ref.setSelectionRange(shownCaret, shownCaret);
      ref.focus();
      autosize();
    });
  };

  const assets = editorAssets({
    id: props.id, element: () => ref, mounted: () => editorMounted,
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
  const { uploadAsset, capturePhotoCmd, voiceMemoToggle, desktopVoiceMemoToggle, createDrawioDiagram, pasteClipboardFiles } = assets;

  let pasteRaw = false;
  let pasteRawToken = 0;
  let pasteRawTimer: number | undefined;
  const clearPasteRaw = () => {
    pasteRaw = false;
    pasteRawToken += 1;
    if (pasteRawTimer !== undefined) {
      window.clearTimeout(pasteRawTimer);
      pasteRawTimer = undefined;
    }
  };
  onCleanup(clearPasteRaw);

  const pasteLiteralText = (text: string) => {
    const start = ref.selectionStart;
    const newRaw = ref.value.slice(0, start) + text + ref.value.slice(ref.selectionEnd);
    commit(newRaw);
    const pos = start + text.length;
    queueMicrotask(() => {
      syncTextareaToStore(ref, newRaw, textareaValue(), pos);
      autosize();
    });
  };

  const selectAc = (item: AcItem) => {
    const t = ac();
    if (!t) return;
    if (item.propertyName) {
      const key = propertyKeyFold(item.propertyName);
      const inserted = `${key}:: `;
      const result = applyCompletion(ref.value, t.start, t.end, inserted);
      const valueTrigger: Trigger = {
        kind: "property-value",
        query: "",
        start: result.caret,
        end: result.caret,
        property: key,
      };
      commit(result.raw);
      setPropertyValueKey(key);
      setAc(valueTrigger);
      setAcIndex(0);
      setAcItems(propertyValueItems(key, ""));
      queueMicrotask(() => {
        ref.value = result.raw;
        ref.setSelectionRange(result.caret, result.caret);
        ref.focus();
        autosize();
        updateAcRect(); // the value popup stays open: re-anchor on the settled caret
      });
      return;
    }
    if (item.propertyValue !== undefined) {
      replaceTrigger(item.propertyValue);
      return;
    }
    if (item.blockRef) {
      const { uuid, externalId, page, kind } = item.blockRef;
      const binding = captureBinding();
      const trigger = ac();
      const editorValue = ref.value;
      let inserted = false;
      const intended = () => bindingCurrent(binding) && ac() === trigger && ref.value === editorValue;
      void persistBlockRefTarget(uuid, page, kind, undefined, externalId, (stampedId) => {
        if (!bindingCurrent(binding) || ac() !== trigger || ref.value !== editorValue) return null;
        const sourcePage = docNode(props.id)?.page;
        if (!sourcePage) return null;
        replaceTrigger(`((${stampedId}))`);
        inserted = true;
        return sourcePage;
      }, intended).then((saved) => {
        if (!saved && bindingCurrent(binding) && (inserted || ac() === trigger))
          pushToast(msg("editor-block-could-not-save-the-block-reference-resolve-the-page-save-and-try-again"), "error");
      }).catch((error) => { if (bindingCurrent(binding)) pushToast(msg("editor-block-could-not-save-the-block-reference", { detail: String(error) }), "error"); });
      return;
    }
    if (item.plugin) {
      const textarea = ref;
      const before = textarea.value;
      const selectionStart = textarea.selectionStart;
      const selectionEnd = textarea.selectionEnd;
      const node = docNode(props.id);
      if (!node) return;
      let depth = 0;
      let parentId = node.parent;
      while (parentId && docNode(parentId) && depth < 1_000) {
        depth++;
        parentId = docNode(parentId).parent;
      }
      const plugin = item.plugin;
      const ownedBlock = bindPluginBlockSnapshot({
        id: node.id,
        raw: node.raw,
        parentId: node.parent,
        depth,
        format: pageByName(node.page)?.format === "org" ? "org" : "md",
      });
      if (!ownedBlock) return;
      const token = ++pluginSlashInvocation;
      const capturedEditingId = editingId();
      const capturedEditingOwner = editingOwner();
      const capturedEditingSurface = editingSurface();
      const capturedSurfaceKey = surfaceKey;
      const trigger = { ...t };
      const editorIsCurrent = () => {
        const liveTrigger = detectTrigger(textarea.value, textarea.selectionStart, propertyValueKey(), pageFmt());
        const liveNode = docNode(props.id);
        return editorMounted
          && token === pluginSlashInvocation
          && isPluginGraphOwnerCurrent(ownedBlock.owner)
          && ref === textarea
          && textarea.isConnected
          && editingId() === capturedEditingId
          && editingOwner() === capturedEditingOwner
          && editingSurface() === capturedEditingSurface
          && surfaceKey === capturedSurfaceKey
          && textarea.value === before
          && textarea.selectionStart === selectionStart
          && textarea.selectionEnd === selectionEnd
          && !!liveTrigger
          && liveTrigger.kind === trigger.kind
          && liveTrigger.start === trigger.start
          && liveTrigger.end === trigger.end
          && liveTrigger.query === trigger.query
          && liveNode?.id === ownedBlock.block.id
          && liveNode.raw === ownedBlock.block.raw;
      };
      closeAc();
      void pluginManager
        .invokeSlashCommand(plugin.pluginId, plugin.contributionId, ownedBlock)
        .then((effects) => {
          if (!editorIsCurrent()) {
            if (isPluginGraphOwnerCurrent(ownedBlock.owner) && textarea.isConnected && textarea.value !== before) {
              pushToast(msg("editor-block-plugin-result-was-not-inserted-because-the-block-changed-while-it-ran"), "info");
            }
            return;
          }
          if (textarea.value !== before) {
            pushToast(msg("editor-block-plugin-result-was-not-inserted-because-the-block-changed-while-it-ran"), "info");
            return;
          }
          const effect = effects.find((candidate) => candidate.kind === "insert-at-caret");
          const text = effect?.kind === "insert-at-caret" ? effect.text : plugin.insertText;
          if (text === undefined) return;
          const result = applyCompletion(before, t.start, t.end, text);
          commit(result.raw);
          queueMicrotask(() => {
            if (!editorMounted
              || token !== pluginSlashInvocation
              || !isPluginGraphOwnerCurrent(ownedBlock.owner)
              || ref !== textarea
              || !textarea.isConnected
              || editingId() !== capturedEditingId
              || editingOwner() !== capturedEditingOwner
              || editingSurface() !== capturedEditingSurface
              || textarea.value !== result.raw) return;
            textarea.value = result.raw;
            textarea.setSelectionRange(result.caret, result.caret);
            textarea.focus();
            autosize();
          });
        })
        .catch((error) => pushToast(msg("editor-block-plugin-slash-command-failed", { detail: String(error) }), "error"));
      return;
    }
    if (item.templateNodes) {
      // Drop the "/name" trigger text, then insert the template's blocks (with
      // dynamic vars resolved). If the host block is now empty, replace it.
      const r = applyCompletion(ref.value, t.start, t.end, "");
      commit(r.raw);
      closeAc();
      const nodes = item.templateNodes.map((n) => templateToOutline(n, docNode(props.id)?.page));
      const wasEmpty =
        docNode(props.id).raw.trim() === "" && docNode(props.id).children.length === 0;
      const lastId = insertOutlineAfter(props.id, nodes);
      if (!lastId) { pushToast(msg("editor-block-outline-is-too-deep-to-insert"), "error"); return; }
      if (wasEmpty) deleteBlock(props.id);
      startEditing(lastId, docNode(lastId).raw.length);
      return;
    }
    switch (item.action) {
      case "task-marker": {
        if (!item.taskMarker) return;
        // OG clears the slash query, sets/replaces the leading task marker, and
        // then moves to the end. Treat this as one semantic edit: literal
        // insertion leaves the old marker intact when invoked mid-block (GH
        // #225) and also bypasses the shared marker grammar.
        const removed = applyCompletion(ref.value, t.start, t.end, "");
        const next = setMarker(removed.raw, item.taskMarker);
        commit(next);
        closeAc();
        queueMicrotask(() => {
          syncTextareaToStore(ref, next, textareaValue(), next.length);
          ref.focus();
          autosize();
        });
        return;
      }
      case "heading-auto":
      case "heading-1":
      case "heading-2":
      case "heading-3":
      case "heading-4": {
        const state = item.action === "heading-auto"
          ? true
          : Number(item.action.slice("heading-".length)) as 1 | 2 | 3 | 4;
        const removed = applyCompletion(ref.value, t.start, t.end, "");
        withUndoUnit(`heading:${props.id}`, [node().page], () => {
          commit(removed.raw);
          setHeading(props.id, state);
        });
        closeAc();
        queueMicrotask(() => {
          const visible = splitProps(node().raw, hideFn(), pageFmt()).visible;
          ref.value = visible;
          const caret = Math.min(visible.length, removed.caret + (state === true ? 0 : state + 1));
          ref.setSelectionRange(caret, caret);
          ref.focus();
          autosize();
        });
        return;
      }
      case "scheduled":
      case "deadline": {
        replaceTrigger("");
        const r = ref.getBoundingClientRect();
        openDatePicker(props.id, item.action, r.left, r.bottom + 4, resultTarget(props.id));
        return;
      }
      case "now-time":
        replaceTrigger(timeStamp());
        return;
      case "youtube-timestamp": {
        replaceTrigger(youtubeTimestampMacroFor(ref) ?? "");
        return;
      }
      case "page-reference":
        // Page reference is a chained command: no GH #35 continuation space,
        // then the ordinary trigger detector owns the blank page lifecycle.
        replaceTrigger("[[]]", 2);
        queueMicrotask(() => void updateAutocomplete());
        return;
      case "insert-link": {
        const removed = applyCompletion(ref.value, t.start, t.end, "", 0);
        const edit = insertLink(removed.raw, t.start, t.start, pageFmt());
        commit(edit.text);
        closeAc();
        queueMicrotask(() => {
          ref.value = edit.text;
          ref.setSelectionRange(edit.start, edit.end);
          ref.focus();
          autosize();
        });
        return;
      }
      case "query-builder": {
        // Insert an empty query, commit it, and drop straight to the rendered
        // view so the visual builder appears — then flag this block so the
        // builder opens its sheet on the empty condition list. The field chooser stays CLOSED: opening it is an
        // explicit user action (Martin 2026-10-03, GH #619 comment 2: a properties dialog that opens by itself
        // "without doing anything" reads as a bug).
        const r = applyCompletion(ref.value, t.start, t.end, QUERY_MACRO_SCAFFOLD);
        commit(r.raw);
        closeAc();
        setQueryBuilderAutoOpen(props.id);
        endEdit("query-builder");
        return;
      }
      case "code-block": {
        // Keep the familiar complete fence scaffold, but open the language
        // picker immediately even though an empty hand-typed fence stays quiet.
        const scaffold = "```\n\n```";
        const result = applyCompletion(ref.value, t.start, t.end, scaffold, 3);
        commit(result.raw);
        openFenceLanguagePicker(result.raw, result.caret);
        return;
      }
      case "page-props": {
        replaceTrigger("");
        const rect = ref.getBoundingClientRect();
        openPageProps(docNode(props.id).page, rect.left, rect.bottom + 4);
        return;
      }
      case "sheet-grid":
      case "sheet-table":
      case "sheet-board": {
        const view = item.action === "sheet-grid" ? "grid" : item.action === "sheet-table" ? "table" : "board";
        replaceTrigger("");
        closeAc();
        applySheetViewSlashAction(props.id, view);
        return;
      }
      case "date-picker":
        replaceTrigger("");
        queueMicrotask(() => { if (editorMounted) openJournalDatePicker(ref, commit, () => editorMounted, autosize); });
        return;
      case "tomorrow":
      case "yesterday":
      case "today":
      case "thatday":
        runJournalSlash(item.action, docNode(props.id).page, replaceTrigger);
        return;
      case "upload-asset":
        replaceTrigger(""); // drop the "/upload" trigger text
        uploadAsset();
        return;
      case "record":
        replaceTrigger(""); // drop the "/record" trigger text
        void desktopVoiceMemoToggle();
        return;
      case "drawio":
        replaceTrigger(""); // drop the "/drawio" trigger text
        void createDrawioDiagram();
        return;
      case "priority-a":
      case "priority-b":
      case "priority-c": {
        // Drop the "/A" trigger, then set the priority token on the first line
        // (placed after any task marker).
        const level: "A" | "B" | "C" =
          item.action === "priority-a" ? "A" : item.action === "priority-b" ? "B" : "C";
        const base = ref.value.slice(0, t.start) + ref.value.slice(t.end);
        const lines = base.split("\n");
        // OG inserts `[#A] ` with a trailing space and moves the caret past it, so
        // the next word or `/command` flows without manually adding a space (the
        // slash menu needs a whitespace boundary before `/`). The space is a
        // live-editing convenience only — `commit` trims it so it never persists.
        lines[0] = setPriority(lines[0], level) + " ";
        const next = lines.join("\n");
        commit(next);
        closeAc();
        const caret = lines[0].length; // after the trailing space
        queueMicrotask(() => {
          ref.value = next;
          ref.setSelectionRange(caret, caret);
          ref.focus();
          autosize();
        });
        return;
      }
    }
    replaceTrigger(item.insert ?? "", item.caret);
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
  const focusNow = () => {
    const historySelection = takeHistoryEditorSelectionFor(props.id, surfaceKey);
    const want = takeCaretFor(props.id);
    const rawCaret = historySelection && !codeBodyProjection(editorValue(), pageFmt()) ? historySelection.start
      : typeof want === "number" ? want : null;
    if (rawCaret !== null) codeView.enter(rawCaret, historySelection?.end ?? rawCaret);
    ref.value = codeShown()?.body ?? (historySelection ? editorValue() : ref.value);
    ref.focus({ preventScroll: true }); // the engine's own focus scroll differs; revealCaretSettled (onMount) owns it (GH #745)
    const v = ref.value;
    if (historySelection) {
      const offset = codeView.mixed() ? codeShown()!.open.length : 0;
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

  let acTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(acTimer));
  const refreshAutocompleteAfterInput = () => {
    // Close the popup synchronously when the trigger ends (instant), but debounce
    // the page/template IPC fetch so holding down a key doesn't fire a backend
    // round-trip per character.
    const next = detectEditorTrigger();
    if (!next) {
      clearTimeout(acTimer);
      closeAc();
      return;
    }
    // Keep replacement span current during debounce: otherwise Enter can accept
    // an old row/range and leave a new suffix behind (`[[Parity Target]]arity Tar`).
    // Rows survive refinement of the same trigger, never a new family/location
    // or a now-blank page/tag lifecycle.
    const previous = ac();
    if (next.kind !== "page" && next.kind !== "tag") releaseCompletionLease();
    // OG save-current-block! withholds references while page-search is active.
    // Preserve autosave and give the read-only picker its pre-trigger baseline.
    if ((next.kind === "page" || next.kind === "tag") &&
        (!previous || previous.kind !== next.kind || previous.start !== next.start)) {
      releaseCompletionLease();
      completionOriginalRaw = rawBeforeLastCommit;
    }
    setAc(next);
    setAcIndex(0);
    setAcBlockState(next.kind === "block" ? "pending" : null);
    if (next.kind === "block") setAcItems([]);
    if (
      !previous ||
      previous.kind !== next.kind ||
      previous.start !== next.start ||
      ((next.kind === "page" || next.kind === "tag") && !next.query.trim())
    ) {
      setAcItems([]);
    }
    clearTimeout(acTimer);
    acTimer = setTimeout(() => void updateAutocomplete(), 90);
  };
  // IMEs replace the textarea's transient composition range many times before
  // committing. Keep that range DOM-local: every setRaw() also dirties the page
  // and starts its save/reparse work. The finalized value has one canonical
  // commit at compositionend instead.
  let compositionActive = false;
  let compositionEndValue: string | null = null;
  // The composing text is not in the store yet, so a page reload from disk must
  // wait for it: hold the page for the composition (a deferred reload replays
  // when the hold is released).
  let releaseCompositionHold: (() => void) | null = null;
  const endCompositionHold = () => {
    releaseCompositionHold?.();
    releaseCompositionHold = null;
  };
  onCleanup(endCompositionHold);
  const beginComposition = () => {
    compositionActive = true;
    compositionEndValue = null;
    clearTimeout(acTimer);
    releaseCompositionHold ??= pinPageWhileDrafting(() => docNode(props.id)?.page ?? null);
  };
  const onCompositionStart = () => beginComposition();
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
    if (compositionEndValue === ref.value) {
      compositionEndValue = null;
      return;
    }
    compositionEndValue = null;
    // Editor keystroke post-processing on a single inserted char (not paste/IME/
    // delete). All branches edit ref.value BEFORE commit so the store sees it.
    if (e.inputType === "insertText" && e.data && e.data.length === 1 && !e.isComposing) {
      const ch = e.data;
      let handled = false;
      // OG parity: typing `::` at the beginning of a property line places the
      // caret before the delimiter. Subsequent property-name characters are
      // authored as `name|::`, not the malformed `::name` that caused GH #306.
      if (ch === ":" && ref.selectionStart >= 2) {
        const caret = ref.selectionStart;
        const lineStart = ref.value.lastIndexOf("\n", caret - 3) + 1;
        if (caret - lineStart === 2 && ref.value.slice(lineStart, caret) === "::") {
          ref.setSelectionRange(lineStart, lineStart);
          handled = true;
        }
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
      const valueKey = propertyValueKeyAfterBoundary(ref.value, ref.selectionStart, ch);
      if (valueKey) setPropertyValueKey(valueKey);
      // OG's typing trigger is exact: only the complete visible editor value
      // `1. ` becomes own numbered-list state, then the trigger text disappears
      // (`src/main/frontend/handler/editor.cljs:1888-1892`, 6e7afa8eb).
      if (ch === " " && ref.value === "1. " && makeOwnNumberedList(props.id, "")) {
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
  // reorder briefly blurs the textarea; cross-day it remounts).
  const moveBlockCmd = (e: KeyboardEvent, dir: 1 | -1): boolean => {
    e.preventDefault();
    const movedEditor = ref;
    const selection = { start: ref.selectionStart, end: ref.selectionEnd, direction: ref.selectionDirection };
    const restore = () => restoreMovedSelection(ref, props.id, selection.start, selection.end, selection.direction);
    commit(ref.value);
    void withBlockMoving(docNode(props.id)?.page ?? "", async () => {
      startEditing(props.id, selection, null, editSurface());
      const intentIsCurrent = captureEditIntent();
      // A sibling reorder happens synchronously (a feed move's own sync part)
      // and keeps this textarea. Restore it in the same gesture: waiting a
      // frame lets Android dismiss the IME despite the later focus.
      const move = outlineScope && !outlineScope.navOnly ? moveItem(props.id, dir) : moveBlockFeed(props.id, dir);
      if (intentIsCurrent() && ref === movedEditor && movedEditor.isConnected && editingId() === props.id
        && (documentOf(movedEditor).activeElement === movedEditor
          || documentOf(movedEditor).activeElement === documentOf(movedEditor).body)) restore();
      const moved = await move;
      // GH #641: a move this surface cannot show says where the block went.
      if (moved !== false && moved !== "none") noticeMoveOutOfView(props.id, outlineScope);
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
    selectBlock(props.id, outlineScope);
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
    "editor/comment": (e) => { e.preventDefault(); e.stopPropagation(); if (!cap) commentFromEditor(props.id, ref, commit, editSurface()); return true; },
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
      selectBlockSubtree(props.id, outlineScope);
      return true;
    },
    "editor/cycle-todo": (e) => { e.preventDefault(); cycleTodoCmd(); return true; },
    "editor/indent": (e) => {
      e.preventDefault();
      // First-line Markdown markers keep OG outline Tab semantics (GH #632).
      // Later in-block list lines and Org retain their list-item nudges.
      const ll = listLine(ref.value, ref.selectionStart);
      if (ll && (pageFmt() === "org" || ll.lineStart > 0)) { nudgeListItem(ll, +2); return true; }
      if (!outlineScope?.navOnly && outlineScope?.roots.includes(props.id)) return true;
      const selection = { start: ref.selectionStart, end: ref.selectionEnd, direction: ref.selectionDirection };
      commit(ref.value);
      if (indentBlock(props.id, selection, editSurface()) === false) pushToast(msg("editor-block-outline-is-too-deep-to-indent"), "error");
      return true;
    },
    "editor/outdent": (e) => {
      e.preventDefault();
      const ll = listLine(ref.value, ref.selectionStart);
      if (ll && (pageFmt() === "org" || ll.lineStart > 0) && ll.indent.length > 0) { nudgeListItem(ll, -2); return true; }
      if (outlineScope?.forceExpandedRoot === docNode(props.id)?.parent) return true;
      const selection = { start: ref.selectionStart, end: ref.selectionEnd, direction: ref.selectionDirection };
      commit(ref.value); outdentBlock(props.id, selection, editSurface()); return true;
    },
  };
  const mobileKeyEvent = { preventDefault() {} } as KeyboardEvent;
  const dispatchMobileEditorCommand = (command: MobileEditorCommandId): boolean => {
    if (!ref || !ref.isConnected) return false;
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
    }
  };
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

  const sheetFaceGridId = (id: string): string | null => {
    if (blockIsGridView(id)) return id;
    return (docNode(id)?.children ?? []).find((child) => blockIsGridView(child)) ?? null;
  };
  const sheetVisibleLength = (id: string): number =>
    splitProps(docNode(id)?.raw ?? "", SHEET_CELL_HIDE).visible.length;
  const deepestLastSheetOutline = (id: string): string => {
    let cur = id;
    for (;;) {
      if (sheetFaceGridId(cur)) return cur;
      const children = docNode(cur)?.children ?? [];
      if (!children.length) return cur;
      cur = children[children.length - 1];
    }
  };
  const nextSheetOutline = (id: string, hostId: string): string | null => {
    if (!sheetFaceGridId(id)) {
      const firstChild = docNode(id)?.children[0];
      if (firstChild) return firstChild;
    }
    let cur = id;
    while (cur !== hostId) {
      const parent = docNode(cur)?.parent ?? null;
      if (!parent) return null;
      const siblings = docNode(parent)?.children ?? [];
      const idx = siblings.indexOf(cur);
      if (idx >= 0 && idx + 1 < siblings.length) return siblings[idx + 1];
      cur = parent;
    }
    return null;
  };
  const prevSheetOutline = (id: string, hostId: string): string | "host" | null => {
    const parent = docNode(id)?.parent ?? null;
    if (!parent) return null;
    const siblings = docNode(parent)?.children ?? [];
    const idx = siblings.indexOf(id);
    if (idx > 0) return deepestLastSheetOutline(siblings[idx - 1]);
    if (parent === hostId) return "host";
    return parent;
  };

  const handleSheetCellKey = (e: KeyboardEvent, start: number, end: number, raw: string): boolean => {
    if (!sheetCell) return false;
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
        const prev = prevSheetOutline(props.id, hostId);
        if (prev === "host") commitAndStartSheetEdit(hostId, sheetVisibleLength(hostId));
        else if (prev) commitAndStartSheetEdit(prev, sheetVisibleLength(prev));
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
          ? props.id === hostId
            ? docNode(hostId)?.children[0] ?? null
            : nextSheetOutline(props.id, hostId)
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
    if (plain && !e.shiftKey && start === end && e.key === "ArrowLeft" && start === 0) {
      e.preventDefault();
      commitAndMove("left");
      return true;
    }
    if (plain && !e.shiftKey && start === end && e.key === "ArrowRight" && start === raw.length) {
      e.preventDefault();
      commitAndMove("right");
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

  const onKeyDown = (e: KeyboardEvent) => {
    // IME owns its key events until compositionend commits the finalized value.
    if (compositionActive || e.isComposing || e.keyCode === 229) return;

    if (codeView.leaveOnKey(e, ref)) { autosize(); return; }
    const start = ref.selectionStart;
    const end = ref.selectionEnd;
    const raw = ref.value;
    // Empty code payloads can leave their wrapper through the existing delete door.
    if (e.key === "Backspace" && !e.ctrlKey && !e.metaKey && !e.altKey && !sheetCell && codeShown() && raw === "" && node().children.length === 0) {
      e.preventDefault();
      const adjacent = [prevVisible(props.id, structuralScope), nextVisible(props.id, structuralScope)].find(id => id && docNode(id)?.page === node().page);
      if (adjacent) {
        deleteBlock(props.id);
        startEditing(adjacent, 0, null, editSurface());
      } else { setRaw(props.id, "", { timetracking: false }); }
      return;
    }

    // Ctrl/Cmd+Shift+V is Logseq's universal raw-paste gesture.
    // ClipboardEvent does not expose modifier keys, so remember the preceding
    // keydown briefly and consume it in onPaste. Do not preventDefault: the
    // platform still has to perform the native clipboard read and dispatch paste.
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && shortcutKey(e) === "v") {
      clearPasteRaw();
      pasteRaw = true;
      const token = ++pasteRawToken;
      pasteRawTimer = window.setTimeout(() => {
        if (pasteRawToken === token) clearPasteRaw();
      }, 1_000);
      return;
    }
    if (pasteRaw) clearPasteRaw();

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
    if (sheetCell && cmd && SHEET_CELL_BLOCKED_EDITOR_COMMANDS.has(cmd)) {
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
      const hostMacro = embedNavExit
        ? detectMacro(docNode(embedNavExit.hostBlockId)?.raw ?? "", formatForBlockId(embedNavExit.hostBlockId)) : null;
      const foldedEmbedHost = embedNavExit && props.id === embedNavExit.firstRoot()
        && isBlockEmbedMacro(hostMacro)
        && collapseSurface?.collapsed(props.id, node().collapsed)
        ? embedNavExit.hostBlockId : null;
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
          let newId: string | null = props.id;
          withUndoUnit(`multiline-exit:${props.id}`, [node().page], () => {
            commit(trimmed);
            newId = insertOutlineAfter(props.id, [{ raw: "", children: [] }]);
          });
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
      commit(raw); // flush current text
      // A comment keeps its quote: Enter in its text adds a reply (a child).
      if (!isAnnot() && start === end && replyOnEnter(props.id, raw, start, editSurface())) return;
      if (isAnnot()) {
        // A highlight block isn't split (that would mangle its metadata); Enter
        // adds a new sibling bullet below, which the user can Tab to nest as a
        // note under the highlight.
        const newId = insertOutlineAfter(props.id, [{ raw: "", children: [] }]);
        if (newId) startEditing(newId, 0, null, editSurface());
      } else {
        const zoomRoot = outlineScope?.forceExpandedRoot === props.id;
        splitBlock(props.id, start, zoomRoot, zoomRoot, editSurface());
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
        if (isAnnot() || isCalc() || codeShown()) return;
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
        const next = nextVisible(props.id, structuralScope);
        if (n && isEditorEmpty(n.raw, hideFn(), pageFmt()) && n.children.length === 0 && next && docNode(next)?.page === n.page) {
          e.preventDefault();
          deleteBlock(props.id);
          startEditing(next, 0, null, editSurface());
        }
      }
    } else if (e.key === "Delete" && end === start && start === raw.length) {
      // GH #213: forward-delete merges with the NEXT block — the mirror of
      // Backspace's merge with the previous one. Never merge a highlight, calc,
      // or body-only code block itself (same rule as Backspace), and never
      // absorb an annotation/calc block's raw text into this one.
      if (isAnnot() || isCalc() || codeShown()) return;
      const next = nextVisible(props.id, structuralScope);
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
    } else if (e.key === "ArrowLeft" && !e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey) {
      // GH #213: at the very start, move into the END of the previous visible
      // editor. Shift keeps native selection; Ctrl/Meta keep native word/line jumps.
      if (start === end && start === 0) {
        const prev = viewPrev();
        if (prev) {
          e.preventDefault();
          // A number caret clamps to the new editor's full text length at mount.
          startEditing(prev, Number.MAX_SAFE_INTEGER, null, navigationSurface());
        }
      }
    } else if (e.key === "ArrowRight" && !e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey) {
      // GH #213: at the very end, move into the START of the next visible editor.
      if (start === end && start === raw.length) {
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
        if (exitingEmbed) prev = prevVisible(embedNavExit!.hostBlockId);
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
      selectBlock(props.id, outlineScope); // exit editing into block-selection mode
    }
  };

  const onBlur = (e: FocusEvent) => {
    clearPasteRaw();
    // The palette must retain the source occurrence until its contextual
    // Select embedding block action runs. Blur still commits the draft.
    if (embedNavExit && (e.relatedTarget as HTMLElement | null)?.closest?.(".switcher")) {
      commit(ref.value);
      savedSel = { start: ref.selectionStart, end: ref.selectionEnd };
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
      savedSel = { start: ref.selectionStart, end: ref.selectionEnd };
      return;
    }
    // Android can blur the WebView editor before document.hasFocus() reflects
    // that the external camera/file-picker activity covered the app. Still the
    // same edit transaction: keep its identity and caret until durable import
    // and insertion finish (GH #493/#622), including a delayed return blur.
    // Graph/block changes stay guarded by assetEditorIsCurrent after the await.
    if (assets.holdsBlur()) {
      commit(ref.value);
      savedSel = { start: ref.selectionStart, end: ref.selectionEnd };
      return;
    }
    // The editor's own window lost focus (switched to another app or window): stay
    // in edit mode and remember the caret so onWindowFocus can resume exactly here.
    // Commit as-is — we're still editing, not exiting. If the window that gains
    // focus next is another Tine window, onOtherWindowFocus ends the edit there.
    if (!documentHasFocus(ref)) {
      commit(ref.value);
      savedSel = { start: ref.selectionStart, end: ref.selectionEnd };
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
    commit(calcExit || codeShown() || handoff ? ref.value : normalizePlanning(ref.value, pageFmt()), calcExit ? { calc: true } : undefined);
    if (!handoff) finishPageHeaderEdit(props.id);
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

  // When the window regains focus, re-focus this block's editor and restore the
  // caret — WebKitGTK drops the native focus on window switch and doesn't put it
  // back. Guarded so we never steal focus if editing moved on while we were away.
  const onWindowFocus = () => {
    if (editingId() !== props.id || !ref || !ref.isConnected) return;
    ref.focus();
    resizeNow(); // a window shown after being hidden may have fit to a 0-height layout
    if (savedSel) {
      const end = Math.min(savedSel.end, ref.value.length);
      const start = Math.min(savedSel.start, end);
      ref.setSelectionRange(start, end);
      savedSel = null;
    }
  };
  // Another Tine window gained focus while this editor's window had lost it: the
  // user moved to a different workspace window, so the edit ends (one editing
  // transaction app-wide). Switching to another application keeps editing.
  const onOtherWindowFocus = () => {
    if (editingId() !== props.id || !ref || !ref.isConnected) return;
    if (documentHasFocus(ref)) return;
    savedSel = null;
    exitEditOnBlur();
  };
  onMount(() => onCleanup(onEachWindow((win) => listen(win, "focus", () => {
    if (win === windowOf(ref)) onWindowFocus();
    else onOtherWindowFocus();
  }))));

  // Paste copied files/images as graph assets. Native file lists use path-based
  // imports; browser-only file payloads use a bounded byte fallback. Ordinary
  // text keeps the existing structural/outline/link behavior below.
  const onPaste = (e: ClipboardEvent) => {
    // Consume the modifier latch on EVERY paste, including file/image pastes.
    // Otherwise an intercepted shortcut could affect a later context-menu paste.
    const rawPaste = pasteRaw;
    clearPasteRaw();
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
    const clipboardItems = Array.from(e.clipboardData?.items ?? []);
    const eventFiles = clipboardItems
      .filter((item) => item.kind === "file")
      .map((item) => item.getAsFile())
      .filter((file): file is File => file !== null);
    const clipboardTypes = Array.from(e.clipboardData?.types ?? []);
    const hasFileFlavor =
      eventFiles.length > 0 ||
      clipboardTypes.some((type) =>
        type === "Files" || type === "text/uri-list" || type === "x-special/gnome-copied-files"
      );
    if (hasFileFlavor) {
      e.preventDefault();
      void pasteClipboardFiles(eventFiles);
      return;
    }
    const start = ref.selectionStart;
    const syntaxSensitive = sheetCell || isCalc() || codeShown() !== null || caretInFence(ref.value, start, pageFmt())
      || caretOnOpeningFence(ref.value, start, pageFmt()) || caretInDisplayMath(ref.value, start, pageFmt());
    const slot = peekClipboardSlot();
    if (!syntaxSensitive) {
      if (slot && text !== "" && normalize(text) === normalize(slot.text)) {
        e.preventDefault();
        // Association is intentionally text-only and can replay the user's last
        // private block copy when a foreign clipboard happens to contain equal
        // normalized text. Identity remains separately one-shot and validated.
        void readOwned(graphOwner(() => editorMounted), pasteClipboardPayload(props.id, slot))
          .then((result) => {
            if (result.kind === "stale") return; const lastId = result.value;
            if (lastId && docNode(lastId)) startEditing(lastId, docNode(lastId).raw.length);
          })
          .catch((error) => reportUiFailure("clipboard-association", error));
        return;
      }
    }
    // A non-empty observed replacement makes stale private data unusable even
    // if a later external clipboard happens to restore the old text. Matching
    // text in a syntax-sensitive surface is only a bypass, not a replacement.
    if (slot && text !== "" && normalize(text) !== normalize(slot.text)) clearClipboardSlot();
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
    const videoMacro = videoPasteMacro(text);
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
    // Single-line/no text: maybe an image on the OS clipboard. Two paths:
    //   1) The paste event's OWN image data (a DataTransferItem of kind "file",
    //      type image/*). Chromium (Windows WebView2) and WKWebView (macOS)
    //      expose it here and normalize Windows' CF_DIBV5/bitmap formats — the
    //      ones screenshot tools like PixPin emit (#43) — into a clean PNG far
    //      more reliably than the Rust arboard plugin does. Read it directly.
    //   2) Linux WebKitGTK does NOT expose image data in the paste event, so
    //      fall back to reading the OS clipboard via the Tauri plugin.
    // The DataTransfer is only valid synchronously, so grab the File now (its
    // reference stays live for the async arrayBuffer read).
    // No image File in the event → try the OS clipboard (the Linux path). Show an
    // immediate "Pasting image…" hint when the clipboard clearly holds one (so
    // there's no dead 1–2s), then render it instantly from the in-memory bytes
    // and write to disk in the background.
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
      if (asChildren) {
        const inserted = withUndoUnit(tag, [pageAtPaste], () => {
          commit(rawAtPaste);
          return insertOutlineChildren(props.id, prepared) ?? false;
        });
        if (!inserted) pushToast(msg("editor-block-pasted-outline-was-refused"), "error");
        return;
      }
      const lastId = withUndoUnit(tag, [pageAtPaste], () => {
        commit(rawAtPaste);
        const host = docNode(props.id);
        const wasEmpty = rawAtPaste.trim() === "" && isEditorEmpty(host.raw, hideFn(), pageFmt()) && host.children.length === 0;
        return (wasEmpty
          ? replaceEmptyBlockWithOutline(props.id, prepared)
          : insertOutlineAfter(props.id, prepared)) ?? false;
      });
      if (!lastId) { pushToast(msg("editor-block-pasted-outline-was-refused"), "error"); return; }
      if (docNode(lastId)) startEditing(lastId, docNode(lastId).raw.length);
    };
    const prepared = sanitizeOutlineIdsForPaste(props.id, nodes);
    if (prepared instanceof Promise) void prepared.then(insert);
    else insert(prepared);
  }
  return (
    <div class="editor-wrap" classList={{ "calc-wrap": isCalc(), "code-wrapping": codeEditing() && codeWrapping() }}>
      <Show when={mixedCodeContext()?.before}>{before => <div class="block-content mixed-code-before" onMouseDown={event => editMixedContext(event, "before")}><AstBody raw={before()} format={pageFmt()} headingLevel={effectiveHeadingLevel(propertySession.facets(node().raw, pageFmt()), depthOf(props.id))} /></div>}</Show>
      <div class="code-editor-region" classList={{ "mixed-code-region": !!mixedCodeContext() }}>
      <Show when={codeEditing()}>
        <span class="code-language">{(codeShown() ?? codeFenceOnly(editorValue(), pageFmt()))?.lang.toLowerCase()}</span>
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
        onCompositionStart={onCompositionStart}
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
      <Show when={mixedCodeContext()?.after}>{after => <div class="block-content mixed-code-after" onMouseDown={event => editMixedContext(event, "after")}><AstBody raw={after()} format={pageFmt()} /></div>}</Show>
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
              onClick={() => commentFromEditor(props.id, ref, commit, editSurface())}>
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

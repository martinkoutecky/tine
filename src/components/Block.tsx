import { Show, Switch, Match, For, createMemo, createSignal, createContext, useContext, createUniqueId, createEffect, onMount, onCleanup, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { autocompleteFacets, backend } from "../backend";
import { reportUiFailure } from "../uiFailure";
import { clearClipboardSlot, normalize, peekClipboardSlot } from "../clipboard";
import {
  detectTrigger,
  applyCompletion,
  refCompletionEnd,
  withRefCompletionSpace,
  autoPairEdit,
  fullWidthRefReplace,
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
  type Trigger,
} from "../editor/autocomplete";
import { navigationName } from "../pageIndex";
import { pluginManager } from "../plugins/manager";
import { bindPluginBlockSnapshot, isPluginGraphOwnerCurrent } from "../plugins/ownership";
import { autoPairInsertOnInput, wrapSelectionEdit, doubleRefKind, backspacePairEdit, SELECTION_WRAP } from "../editor/autopair";
import { typoTypeReplace } from "../render/typography";
import { linkAutocompletePolicy } from "../editor/linkDefault";
import { spellcheckEnabled } from "../spellcheckSettings";
import { restoreMovedSelection } from "../editor/restoreMovedSelection";
import { spaceAfterRefCompletion } from "../refCompletionSettings";
import { pageByName, blockPageReadOnly, setRaw, setBlockProperty, makeOwnNumberedList, removeOwnNumberedList, stopOwnNumberedListOnEmptyEnter, splitBlock, indentBlock, outdentBlock, mergeWithPrev, mergeWithNext, toggleCollapse, setCollapsed, prevVisible, nextVisible, nextVisibleOrExtend, beginPageHeaderEdit, finishPageHeaderEdit, insertEmptyChildBlock, insertOutlineAfter, replaceEmptyBlockWithOutline, insertOutlineChildren, outlineFits, pasteClipboardPayload, sanitizeOutlineIdsForPaste, deleteBlock, moveBlockFeed, moveItem, selectBlock, selectBlockSubtree, moveSelection, isSelected, persistBlockRefTarget, isBlockMoving, withBlockMoving, orderedListMarker, withUndoUnit, blockIsGridView, trackAssetWrite, formatForBlock, depthOf, setHeading, collapsibleDescendantIds, setCollapsedDescendants, blockExternalId, type OutlineScope, node as docNode } from "../document";
import { openDurableBlock } from "../blockRefActions";
import {
  clearFocusSurface,
  editingId,
  editingOwner,
  editingSurface,
  endEdit,
  focusSurfaceFor,
  noteSurfaceFocused,
  registerHistoryEditorTarget,
  startEditing,
  takeCaretFor,
  takeHistoryEditorSelectionFor,
} from "../editorController";
import { OUTLINE_MAX_SOURCE_CHARS, parseOutline, type OutlineNode } from "../editor/outline";
import { structuredHtmlOutline } from "../editor/htmlPaste";
import {
  toggleInlineFormat,
  insertLink,
  wrapLink,
  isPasteableUrl,
  videoPasteMacro,
  killLineBefore,
  killLineAfter,
  wordForward,
  wordBackward,
  killWordForward,
  killWordBackward,
  setPriority,
  type Edit,
  type InlineFormat,
} from "../editor/format";
import {
  essentialSelectionActions,
  secondarySelectionActions,
  type SelectionAction,
} from "../editor/selectionActions";
import { isRenderHiddenProp, isPropertyLine, propertyKeyNorm } from "../render/block";
import { effectiveHeadingLevel, facetsOf } from "../render/facets";
import { AstBody } from "../render/body";
import { InlineText, CopyButton } from "../render/inline";
import {
  assetMarkdown,
  assetFileName,
  captureAssetFileName,
  recordingExt,
} from "../media";
import { MEDIA_EDITORS } from "../mediaEditors";
import { resolveMediaEditorCommand } from "../mediaEditorSettings";
import { refreshAssetOnReturn } from "../assetRefresh";
import { isMobilePlatform } from "../nativeChrome";
import { runJournalSlash } from "../journalSlash";
import { calcSource, serializeCalcExitCommit, evalCalc } from "../editor/calc";
import { QueryMacro, EmbedMacro, youtubeTimestampMacroFor } from "./Macro";
import { workflow, zoomInto, openContextMenu, openDatePicker, setQueryBuilderAutoOpen, openPageProps, autoPairing, typographyMode, timetrackingEnabled, blockReferencesRequest, documentMode, docModeEnterForNewBlock, searchRemoveAccents } from "../ui";
import { graphMeta, dataRev, graphEpoch } from "../graphSession";
import { pushToast, dismissToast } from "../toasts";
import { copyBlockLink } from "./blockLinkCopy";
import { seedAssetBlob } from "../assetCache";
import { assetEditorIsCurrent, captureAssetEditor, reportStaleAsset, type AssetEditorToken } from "../assetLanding";
import { captureBinding, stillBound } from "../binding";
import { graphOwner, latestOwner, ownedWhen, readOwned, writeOwned } from "../owned";
import { blockRefCount } from "../blockRefCounts";
import { BlockReferences } from "./BlockReferences";
import { editorCommandFor, isPermittedTabGesture, isTabLikeEvent } from "../keybindings";
import { cycleMarkerSmart, markerLabelClickable } from "../editor/repeat";
import { setMarker } from "../editor/marker";
import { registerTransientLayer } from "../transientLayers";
import { taskCheckboxState } from "../markers";
import { applyTemplateVars, prepareTemplateVars } from "../editor/templateVars";
import {
  caretAtFirstRow,
  caretAtLastRow,
  caretColumnOnVisualRow,
  caretOffsetOnLastRow,
  textareaCaretLeft,
} from "../editor/caretRows";
import { splitProps, joinProps, isBuiltinHidden, isSheetCellHidden, hideAll, caretInFence, caretOnPropertyLine, isPropertiesOnly, multilineExitTrim } from "../editor/properties";
import { QUERY_MACRO_SCAFFOLD, queryMacroExtents, singleQueryMacroExtent, type MacroExtent } from "../editor/queryMacro";
import { normalizePlanning } from "../editor/planning";
import { caretOnOpeningFence, caretInDisplayMath } from "../editor/fences";
import { codeBodyExitTrim, codeBodyJoin, codeBodyProjection, codeFenceOnly } from "../editor/codeFence";
import { isAnnotationBlock, annotationInfo } from "../editor/annotation";
import { AnnotationBody } from "./AnnotationBody";
import { logbookInfo, type LogbookInfo } from "../logbook";
import { inPageFindPreservesEditorBlur } from "../inpageFind";
import { registerFocusedEditorCommandBridge, type MobileEditorCommandId } from "../editorCommandBridge";
import {
  isRecordingAudio,
  setRecordingAudio,
  cancelDesktopVoiceRecording,
  desktopVoiceRecordingActive,
  startDesktopVoiceRecording,
  stopDesktopVoiceRecording,
} from "../mediaCapture";
import { sheetConfig } from "../sheet/config";
import { SheetCellContext } from "../sheet/context";
import { appendSheetCellChild, structuralSheetPasteNode } from "../sheet/mutations";
import { cellBlockId, cellOwner, cellSurfaceKey, selectCellAfterEdit, moveCellAfterEdit, selectTopRowSeamAfterEdit } from "../sheet/selection";
import { forbidsEditEntry } from "../editor/editTargets";
import { SheetGrid } from "./SheetGrid";
import { SheetTable } from "./SheetTable";
import { SheetBoard } from "./SheetBoard";
import { blockBackgroundColor } from "../blockColors";
import { blockDtoExternalId } from "../blockIdentity";
import { SheetContainer } from "./SheetContainer";
import { shouldOpenBlockContextMenu } from "../contextMenuPolicy";
import { beginDrag, beginEditGesture, bulletDragMoved, dragId, dropInd, renderedClickOffset } from "./blockGestures";
import { captureEditorScrollAnchor } from "../editor/scrollAnchor";
import { CalGlyph, ClockBadge, blockFirstLine, toggleBlockMarkerLabel, formatForBlockId, listLineAt, nearestScrollableY, timeStamp, toggleBlockCheckbox } from "./blockParts";

type SheetSlashView = "grid" | "table" | "board";

export function applySheetViewSlashAction(id: string, view: SheetSlashView): string | null {
  const node = docNode(id);
  if (!node) return null;
  let seededCellId: string | null = null;
  withUndoUnit(`sheet:view:${view}`, [node.page], () => {
    const shouldSeedGrid = view === "grid" && (docNode(id)?.children.length ?? 0) === 0;
    setBlockProperty(id, "tine.view", view);
    if (view === "board") setBlockProperty(id, "tine.group-by", "state");
    if (shouldSeedGrid) {
      const rowId = insertEmptyChildBlock(id, 0);
      if (rowId) seededCellId = insertEmptyChildBlock(rowId, 0);
    }
  });
  endEdit("select-block");
  if (seededCellId) startEditing(seededCellId, 0);
  return seededCellId;
}

// Detect a block whose entire body is a single {{query}}/{{tine-query}}/{{embed}} macro.
function detectMacro(raw: string): { kind: "query" | "embed"; inner: string; sourceExtent?: MacroExtent } | null {
  // The visible body: property lines stripped so `{{query}}\nid:: …` still matches.
  const text = raw.split("\n").filter((l) => !isPropertyLine(l)).join("\n").trim();
  const [q, ...rest] = queryMacroExtents(text); // shared reader: a second macro or a `}}` in a string never merges
  if (q && !rest.length && q.start === 0 && q.end === text.length) return { kind: "query", inner: `${q.name} ${q.argument}`, sourceExtent: singleQueryMacroExtent(raw, q) };
  const m = /^\{\{(embed)\b([\s\S]*)\}\}$/.exec(text);
  return m ? { kind: "embed", inner: `${m[1]}${m[2]}` } : null;
}

// Any complete {{query …}} macro anywhere in the body. The shared scanner is
// brace/string/page-ref aware and catches inline macros ("Tasks {{query …}}"),
// not only macros occupying their own line.
function bodyContainsQueryMacro(raw: string): boolean {
  return queryMacroExtents(raw).length > 0;
}

// (Rendered-property hidden set lives in render/block.ts as RENDER_HIDDEN_PROPS /
// isRenderHiddenProp, shared with body.tsx's renderProps.)

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
export interface CollapseSurfaceApi {
  collapsed: (id: string, stored: boolean) => boolean;
  toggle: (id: string, current: boolean) => void;
  setMany: (ids: readonly string[], collapsed: boolean) => void;
}
// Deliberate Tine divergence from OG Logseq: OG block embeds use the source
// block's persisted collapsed state, while Tine lets a secondary/transcluded
// rendering fold locally so interacting with a view cannot mutate its source.
// Keep this surface-local contract explicit when changing collapse parity.
export const CollapseSurfaceContext = createContext<CollapseSurfaceApi | null>(null);

/** Render and edit one document block through the document door. Work scales
 * with its visible descendants; a failed structured paste shows fixed text. */
export function Block(props: { id: string; hideRefCount?: boolean; forceExpanded?: boolean }): JSX.Element {
  const node = () => docNode(props.id);
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
  const hasChildren = () => node().children.length > 0;
  const collapsed = () => collapseSurface?.collapsed(props.id, node().collapsed) ?? node().collapsed;
  const collapsibleDescendants = createMemo(() => collapsibleDescendantIds(props.id));
  const hasCollapsedDescendant = createMemo(() =>
    collapsibleDescendants().some((id) => {
      const descendant = docNode(id);
      return descendant
        ? collapseSurface?.collapsed(id, descendant.collapsed) ?? descendant.collapsed
        : false;
    })
  );
  const toggleCollapsedDescendants = () => {
    const ids = collapsibleDescendants();
    if (!ids.length || (readOnly() && !collapseSurface)) return;
    // OG semantics: any folded descendant means “expand all”; only a completely
    // open subtree means “collapse all”. The guide parent itself stays open.
    const next = !hasCollapsedDescendant();
    if (collapseSurface) collapseSurface.setMany(ids, next);
    else setCollapsedDescendants(props.id, next);
  };
  const fmt = () => pageByName(node().page)?.format ?? "md";
  const blockFacets = createMemo(() => {
    const n = node();
    return n ? facetsOf(n.raw, fmt()) : null;
  });
  // A table/board view on a block whose body CONTAINS a {{query}} macro belongs
  // to the query results (the macro path renders it, rowSource: query) — the
  // children-source face here would render a SECOND, empty sheet below it. The
  // macro need not be the whole body: the §4 demo block is a heading +
  // {{query}} + tine.view:: board in ONE block, which the exact-body
  // detectMacro misses. Grid stays children-source even on a query block.
  const sheet = createMemo(() => {
    const cfg = sheetConfig(blockFacets()?.properties ?? []);
    if ((cfg.view === "table" || cfg.view === "board") && bodyContainsQueryMacro(node().raw)) {
      return { ...cfg, view: null };
    }
    return cfg;
  });
  // Heading level of THIS block's first line, so the bullet column can match the
  // (taller) heading line box and the bullet stays centered on it.
  const headingLevel = createMemo(() => {
    const facets = blockFacets();
    return facets ? effectiveHeadingLevel(facets, depthOf(props.id)) : null;
  });
  const editorVisibleValue = createMemo(() => {
    const n = node();
    if (!n) return "";
    const fmt = pageByName(n.page)?.format === "org" ? "org" : "md";
    return splitProps(n.raw, isBuiltinHidden, fmt).visible;
  });
  const editorIsUniline = createMemo(() => !editorVisibleValue().includes("\n"));
  // Block-level "linked references" panel toggled by the reference-count badge.
  const [showRefs, setShowRefs] = createSignal(false);
  createEffect(() => {
    const requested = blockReferencesRequest()?.id;
    if (requested === props.id || requested === blockExternalId(props.id)) setShowRefs(true);
  });
  // Ordered-list label for THIS block's own bullet (OG numbers the block itself,
  // not its children); null for a normal bullet.
  const orderMarker = () => orderedListMarker(props.id);
  // An org page Tine can't round-trip is shown but NOT editable (Tine must never
  // rewrite it). Clicking a block doesn't enter the editor on such a page.
  const readOnly = () => blockPageReadOnly(props.id);
  // A whole-block `{{embed ((uuid))}}` is a transparent host for the referenced
  // outline. Showing both this storage block's controls and the referenced root's
  // controls produces two consecutive bullets. Keep the referenced root controls
  // (they own collapse/zoom/sidebar behavior) and suppress only the macro host.
  const blockEmbedHost = createMemo(() => {
    const m = detectMacro(node().raw);
    return m?.kind === "embed" && /^embed\s*\(\([^)]+\)\)\s*$/i.test(m.inner);
  });

  return (
    <div
      class="ls-block"
      classList={{
        collapsed: collapsed(),
        "block-embed-host": blockEmbedHost(),
        "plugin-thread-lines": pluginManager.hasDeclarativeDecoration("thread-lines"),
        "plugin-thread-lines-active": pluginManager.declarativeDecorationSetting("thread-lines", "display") === "active",
        "plugin-thread-lines-standard": pluginManager.declarativeDecorationSetting("thread-lines", "intensity") === "standard",
      }}
      data-block-id={props.id}
      data-block-ref={blockExternalId(props.id) ?? props.id}
    >
      <div
        class="block-main"
        classList={{
          // Heading level on the row so the bullet column can match the (taller)
          // heading line box and the bullet stays centered on the first line. While
          // editing, apply the same offset only when the hidden-props-stripped editor
          // value is still a single line; multi-line heading blocks edit at body size.
          [`bullet-h${headingLevel()}`]: headingLevel() != null && (!editing() || editorIsUniline()),
          "drop-before": dropInd()?.id === props.id && dropInd()?.before === true,
          "drop-after": dropInd()?.id === props.id && dropInd()?.before === false,
          dragging: dragId() === props.id,
          selected: isSelected(props.id),
          // Marks the row being edited; drives dim-mode's active-block spotlight.
          editing: editing(),
        }}
        onContextMenu={(e) => {
          if (!shouldOpenBlockContextMenu(e.target)) return;
          e.preventDefault();
          openContextMenu(e.clientX, e.clientY, props.id);
        }}
      >
        <Show when={!blockEmbedHost()}>
        <div class="block-controls">
          <span
            class="collapse-toggle"
            classList={{ "has-children": hasChildren(), disabled: readOnly() }}
            aria-disabled={readOnly() ? "true" : undefined}
            onClick={() => {
              if (readOnly()) return;
              if (collapseSurface) collapseSurface.toggle(props.id, collapsed());
              else toggleCollapse(props.id);
            }}
          >
            <Show when={hasChildren()}>
              <svg viewBox="0 0 24 24" class="triangle">
                <path d="M8 5l8 7-8 7z" />
              </svg>
            </Show>
          </span>
          <span
            class="bullet-container"
            classList={{ "bullet-closed": collapsed() && hasChildren(), ordered: !!orderMarker() }}
            title="Click to zoom; shift-click → sidebar; middle-click → new tab; drag to move"
            onMouseDown={(e) => {
              if (e.button === 0 && !readOnly()) beginDrag(props.id, e);
            }}
            onClick={(e) => {
              e.stopPropagation();
              if (bulletDragMoved()) return; // was a drag, not a click
              if (e.shiftKey) void openDurableBlock(props.id, "sidebar");
              else zoomInto(props.id);
            }}
            onAuxClick={(e) => {
              if (e.button !== 1) return; // middle-click → open the zoom in a new tab
              e.preventDefault();
              e.stopPropagation();
              void openDurableBlock(props.id, "tab");
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
          <Show
            when={editing()}
            fallback={
              <Rendered
                id={props.id}
                owner={instanceId}
                outlineScope={outlineScope}
                trailing={
                  // OG's per-block reference-count badge: shown only when the block
                  // is referenced. Plain click toggles the referrers panel below;
                  // shift-click opens the block in the sidebar (matching OG and the
                  // bullet's shift-click).
                  <Show when={blockRefCount(props.id) > 0 && !props.hideRefCount}>
                    <a
                      class="block-refs-count"
                      classList={{ open: showRefs() }}
                      title="Open block references (shift-click → sidebar)"
                      onClick={(e) => {
                        e.stopPropagation();
                        if (e.shiftKey) void openDurableBlock(props.id, "sidebar");
                        else setShowRefs((v) => !v);
                      }}
                    >
                      {blockRefCount(props.id)}
                    </a>
                  </Show>
                }
              />
            }
          >
            <Editor id={props.id} />
          </Show>
        </div>
      </div>

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
              <button
                type="button"
                class="block-children-left-border"
                aria-label={hasCollapsedDescendant() ? "Expand all descendants" : "Collapse all descendants"}
                aria-expanded={!hasCollapsedDescendant()}
                disabled={collapsibleDescendants().length === 0 || (readOnly() && !collapseSurface)}
                title={hasCollapsedDescendant() ? "Expand all descendants" : "Collapse all descendants"}
                onClick={(event) => {
                  event.stopPropagation();
                  toggleCollapsedDescendants();
                }}
              />
              <div class="block-children">
                <For each={node().children}>{(cid) => <Block id={cid} />}</For>
              </div>
            </div>
          </Match>
        </Switch>
      </Show>
    </div>
  );
}

const SHEET_CELL_BLOCKED_EDITOR_COMMANDS = new Set([
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

function Rendered(props: {
  id: string;
  owner?: string;
  trailing?: JSX.Element;
  outlineScope?: OutlineScope | null;
}): JSX.Element {
  const node = () => docNode(props.id);
  const fmt = () => pageByName(node().page)?.format ?? "md";
  // Header facets (marker/priority/heading/scheduled/deadline/properties) off the
  // ONE lsdoc parse — read from the cache the store seeded from the backend DTO (no
  // parse on load), recomputed from a single wasm parse only for the edited block.
  const facets = createMemo(() => facetsOf(node().raw, fmt()));
  const headingLevel = createMemo(() => effectiveHeadingLevel(facets(), depthOf(props.id)));
  const clock = createMemo((): LogbookInfo | null => {
    if (!timetrackingEnabled()) return null;
    const marker = facets().marker;
    if (marker !== "DONE" && marker !== "TODO" && marker !== "LATER") return null;
    const info = logbookInfo(node().raw);
    return info.seconds > 0 ? info : null;
  });
  const readOnly = () => blockPageReadOnly(props.id);

  const macro = createMemo(() => detectMacro(node().raw));

  // PDF highlight (annotation) blocks render a colored, clickable swatch
  // (AnnotationBody) that opens the PDF at the highlight's page; notes go in
  // child blocks. The detection + rendering live in editor/annotation +
  // components/AnnotationBody.
  const annotation = createMemo(() => annotationInfo(facets().properties));
  // The highlight text shown in the annotation swatch = the first visible (non-
  // property) line of the block (cheap; the shared line recognizer).
  const annotationLine = () => node().raw.split("\n").find((l) => !isPropertyLine(l) && l.trim() !== "") ?? "";

  // Click edits the block, placing the caret WHERE you clicked when lsdoc span
  // data can map the rendered leaf back through source bytes and hidden props.
  // Anything without trustworthy span data (chips, macro hosts, parser fallback)
  // keeps the old end-of-block behavior.
  let contentRef: HTMLDivElement | undefined;
  const clickOffset = (e: MouseEvent): number | null =>
    contentRef ? renderedClickOffset(contentRef, node().raw, pageByName(node().page)?.format === "org" ? "org" : "md", e) : null;
  // For annotation blocks the editor shows only the highlight text (metadata
  // stays hidden); the colored prefix still jumps to the PDF.
  //
  // The caret offset must be computed at MOUSEDOWN — before the previously-
  // focused editor blurs and reflows the layout (on click the coordinates are
  // stale; the mouseup can even land on a different element so no block receives
  // the click at all). Whether it becomes an EDIT (click) or a SELECTION (drag)
  // is decided at mouseup — see beginEditGesture.
  const onMouseDown = (e: MouseEvent) => {
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return;
    if (readOnly()) return; // read-only org page — never enter the editor
    if (forbidsEditEntry(e)) return;
    e.stopPropagation(); // keep the row wrapper from arming a second gesture
    beginEditGesture(
      e,
      props.id,
      clickOffset(e) ?? node().raw.length,
      props.owner ?? null,
      props.outlineScope ?? null,
    );
  };

  const displayProps = () => {
    const extra = graphMeta()?.block_hidden_properties ?? [];
    return facets().properties.filter(([k]) => !isRenderHiddenProp(k, extra));
  };
  const bgColor = () => {
    return blockBackgroundColor(facets().properties);
  };

  const body = (
    <Show when={annotation()} fallback={<AstBody raw={node().raw} blockId={props.id} format={fmt()} headingLevel={headingLevel()} />}>
      <AnnotationBody
        highlightId={blockExternalId(props.id) ?? props.id}
        color={annotation()!.color}
        hlPage={annotation()!.hlPage}
        line={annotationLine()}
        page={node().page}
      />
    </Show>
  );

  return (
    <Show
      when={!macro()}
      fallback={
        <div class="block-content macro-host" onMouseDown={onMouseDown}>
          <Switch>
            <Match when={macro()!.kind === "query"}>
              <QueryMacro body={macro()!.inner} blockId={props.id} sourceExtent={macro()!.sourceExtent} sourceRaw={macro()!.sourceExtent && node().raw.slice(macro()!.sourceExtent!.start, macro()!.sourceExtent!.end)} />
            </Match>
            <Match when={macro()!.kind === "embed"}>
              <EmbedMacro body={macro()!.inner} blockId={props.id} />
            </Match>
          </Switch>
        </div>
      }
    >
    <div
      ref={contentRef}
      class="block-content"
      classList={{ done: facets().done, "has-bg": !!bgColor(), [`heading h${headingLevel() ?? ""}`]: headingLevel() != null }}
      style={bgColor() ? { background: bgColor() } : undefined}
      onMouseDown={onMouseDown}
    >
      <Show when={taskCheckboxState(facets().marker) !== null}>
        <span
          class="block-task-checkbox"
          classList={{ checked: taskCheckboxState(facets().marker) === true }}
          role="checkbox"
          aria-checked={taskCheckboxState(facets().marker) === true}
          title={taskCheckboxState(facets().marker) === true ? "Mark undone" : "Mark done"}
          // Mouse-DOWN (not click) + preventDefault so toggling never enters the
          // block editor (OG parity — matches the block-ref/chip mousedown model).
          onMouseDown={(e) => {
            e.stopPropagation();
            e.preventDefault();
            toggleBlockCheckbox(props.id);
          }}
        />{" "}
      </Show>
      <Show when={facets().marker}>
        <span
          class={`block-marker marker-${facets().marker?.toLowerCase()}`}
          classList={{ "marker-clickable": markerLabelClickable(facets().marker) }}
          onClick={(e) => {
            e.stopPropagation();
            toggleBlockMarkerLabel(props.id);
          }}
        >
          {facets().marker}
        </span>{" "}
      </Show>
      <Show when={facets().priority}>
        <span class={`block-priority priority-${facets().priority}`}>[#{facets().priority}]</span>{" "}
      </Show>
      {/* Heading size is applied inside AstBody to ONLY the heading's first line
          (see renderBlocks headingLevel), so a `> quote`/table/etc. continuation in
          the same block renders at normal size — matching OG. */}
      {body}
      <Show when={clock()}>
        {(info) => <ClockBadge info={info()} />}
      </Show>
      <Show when={facets().scheduled}>
        <span
          class="date-chip scheduled"
          title="Scheduled — click to change"
          onClick={(e) => {
            e.stopPropagation();
            openDatePicker(props.id, "scheduled", e.clientX, e.clientY);
          }}
        >
          <CalGlyph /> {facets().scheduled}
        </span>
      </Show>
      <Show when={facets().deadline}>
        <span
          class="date-chip deadline"
          title="Deadline — click to change"
          onClick={(e) => {
            e.stopPropagation();
            openDatePicker(props.id, "deadline", e.clientX, e.clientY);
          }}
        >
          <CalGlyph /> {facets().deadline}
        </span>
      </Show>
      <Show when={displayProps().length > 0}>
        <span class="block-properties">
          <For each={displayProps()}>
            {([k, v]) => (
              <span class="prop">
                <span class="prop-key">{propertyKeyNorm(k)}</span>
                {/* Render the value through the inline parser so a `[[wiki]]`/`#tag`
                    property value becomes a clickable link, matching OG and the
                    page-property path (Page.tsx). Issue #10. */}
                <span class="prop-value"><InlineText text={v} format={fmt()} /></span>
              </span>
            )}
          </For>
        </span>
      </Show>
      {props.trailing}
    </div>
    </Show>
  );
}

interface AcItem {
  label: string;
  /** Secondary, dimmer line (e.g. the page a block-ref candidate lives on). */
  sub?: string;
  insert?: string;
  caret?: number;
  action?: import("../editor/autocomplete").CommandAction;
  taskMarker?: string;
  plugin?: { pluginId: string; contributionId: string; insertText?: string };
  templateNodes?: import("../types").BlockDto[];
  /** A `((block reference))` candidate. `uuid` finds the target; `externalId` is persisted. */
  blockRef?: { uuid: string; externalId: string; page: string; kind: import("../types").PageKind };
  /** Canonical property-name candidate, or a newly folded typed key. */
  propertyName?: string;
  /** Existing or newly typed value for the active canonical property. */
  propertyValue?: string;
}

// and DTO→outline conversion for insertion.
let templateCache: import("../types").TemplateDto[] | null = null;
let templateCacheRev = -1;
let templateCacheEpoch = -1;
async function getTemplates(): Promise<import("../types").TemplateDto[]> {
  // Re-fetch when the graph has changed since the last fetch,
  // so a template just created (here or externally) shows up without a reload.
  const rev = dataRev(), epoch = graphEpoch();
  if (templateCache && templateCacheRev === rev && templateCacheEpoch === epoch) return templateCache;
  const owner = graphOwner(() => dataRev() === rev && graphEpoch() === epoch);
  try {
    const templates = await readOwned(owner, backend().listTemplates());
    if (templates.kind === "stale") return []; templateCache = templates.value;
    templateCacheRev = rev;
    templateCacheEpoch = epoch;
    if (templateCache.length) await prepareTemplateVars();
  } catch { if (owner()) templateCache = []; }
  return templateCache ?? [];
}
function templateToOutline(
  b: import("../types").BlockDto,
  currentPage?: string
): { raw: string; children: any[] } {
  return {
    raw: applyTemplateVars(b.raw, currentPage),
    children: b.children.map((c) => templateToOutline(c, currentPage)),
  };
}
// Block editor for `props.id`. Inside quick capture, CaptureCtx repurposes
// Enter/Escape when autocomplete is closed to commit or dismiss the capture.
export function Editor(props: { id: string }): JSX.Element {
  // Non-null only inside the quick-capture window (see CaptureCtx).
  const cap = useContext(CaptureCtx);
  const sheetCell = useContext(SheetCellContext);
  // Which surface (main pane / a specific sidebar item) this editor lives in —
  // drives edit-focus arbitration when the same block renders in several surfaces.
  const surfaceKey = useContext(SurfaceContext);
  const outlineScope = useContext(OutlineScopeContext);
  const embedNavExit = useContext(EmbedNavExitContext);
  // Generic ref/query surfaces intentionally return structural keyboard edits to
  // the primary outline. An embed is a live editing surface: structural destinations
  // (Enter, Arrow navigation, and empty-block merge/delete) must remain in the
  // transclusion the user is looking at.
  const editSurface = () => surfaceKey.startsWith("embed:") ? surfaceKey : null;
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

  // What the textarea shows. Annotation (PDF highlight) blocks expose only their
  // highlight text (all metadata hidden); every other block hides just the
  // built-in id::/collapsed:: lines (like OG). Hidden lines are preserved and
  // reattached on commit.
  const isAnnot = () => isAnnotationBlock(node().raw);
  // Annotation blocks hide ALL properties (edit only the highlight text); every
  // other block hides just the built-in id::/collapsed::. One fence-aware splitter.
  const hideFn = () => (isAnnot() ? hideAll : sheetCell ? isSheetCellHidden : isBuiltinHidden);
  const editorValue = createMemo(() => splitProps(node().raw, hideFn(), pageFmt()).visible);
  // GH #357: while the buffer IS one whole-block code fence the editor presents
  // as the same mono, no-wrap card the rendered face is (no re-layout jump).
  // Mixed content / ```calc keep their own modes; re-derived per keystroke.
  const codeEditing = createMemo(() => codeFenceOnly(editorValue(), pageFmt()) !== null);
  const editorHeadingLevel = createMemo(() => {
    const visible = editorValue();
    if (visible.includes("\n")) return null;
    return effectiveHeadingLevel(facetsOf(visible, pageFmt()), depthOf(props.id));
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
  const commit = (text: string, opts?: { timetracking?: boolean; calc?: boolean }) => {
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
    if (!commitAsCalc && !codeShown() && text === editorValue()) return;
    // For a code wrapper `text` is the payload body: re-attach the exact wrapper
    // bytes (GH #412/#413: the body-only projection is reversible).
    const visible = commitAsCalc ? serializeCalcExitCommit(text, editorValue()) : codeWrapCommit(text) ?? text;
    const next = joinProps(visible, splitProps(node().raw, hideFn(), pageFmt()).hidden, pageFmt());
    // No-op commit (text that reconstructs the identical raw): don't mark the page
    // dirty or push undo — avoids churn and can't rewrite the block's bytes.
    if (next === node().raw) return;
    const setRawOpts = opts && "timetracking" in opts ? { timetracking: opts.timetracking } : undefined;
    // GH #515: capture once for the autosize frame, before live mirrors above react.
    if (pendingScrollAnchor === undefined) {
      pendingScrollAnchor = ref && document.activeElement === ref
        ? captureEditorScrollAnchor(ref, nearestScrollableY(ref)) : null;
    }
    autosize();
    setRaw(props.id, next, setRawOpts);
  };

  // Nest/un-nest an in-block list item by ±2 leading spaces (Tab/Shift-Tab when
  // the caret is on a `+`/`*`/ordered list line).
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
  const [acIndex, setAcIndex] = createSignal(0);
  // GH #412/#413: inside a COMPLETE whole-block code wrapper the editor shows
  // only the payload between the wrapper lines (fences stay out of the editing
  // surface, and select-all reaches only the payload); commits re-attach the
  // exact raw wrapper bytes via the reversible projection. Mixed content,
  // incomplete wrappers and ```calc keep raw editing. The code-language picker
  // suppresses the swap while open: its query lives on the opener line.
  const codeShown = createMemo(() => (ac()?.kind === "code-language" ? null : codeBodyProjection(editorValue(), pageFmt())));
  const codeWrapCommit = (text: string): string | null => {
    const p = codeShown();
    return p ? codeBodyJoin(p, text) : null;
  };
  // One door to the fence language picker: `/Code block` and the hand-typed ```
  // scaffold both come here (GH #507). While open `codeShown` keeps the raw
  // view so the opener line stays visible; choosing a language, or Escape,
  // then drops into the body-only view.
  const openFenceLanguagePicker = (raw: string, fenceEnd: number) => {
    setAc({ kind: "code-language", query: "", start: fenceEnd, end: fenceEnd });
    setAcIndex(0);
    setAcItems(codeLanguageItems("").map((language) => ({
      label: language.label,
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
  // Keep the highlighted autocomplete item scrolled into view during arrow nav.
  createEffect(() => {
    acIndex();
    queueMicrotask(() =>
      acListRef?.querySelector(".ac-item.active")?.scrollIntoView({ block: "nearest" })
    );
  });

  // The autocomplete popup is rendered through a Portal (fixed-positioned), so a
  // clipping ancestor — the right sidebar's `overflow:auto`, a modal — can't cut
  // it off. We anchor it to the textarea's viewport rect and recompute while it's
  // open (the editor grows as you type) and on scroll/resize.
  const [acRect, setAcRect] = createSignal<{ left: number; top: number; bottom: number } | null>(null);
  const updateAcRect = () => {
    if (ref) {
      const r = ref.getBoundingClientRect();
      setAcRect({ left: r.left, top: r.top, bottom: r.bottom });
    }
  };
  createEffect(() => {
    if (ac() && acItems().length > 0) updateAcRect(); // re-anchor on open / each keystroke
  });
  // Flip the popup above the line when there isn't room below (near the viewport
  // bottom), so it stays fully visible — matches OG's caret-aware placement.
  const acStyle = (): Record<string, string> => {
    const r = acRect();
    if (!r) return {};
    const below = window.innerHeight - r.bottom;
    const openUp = below < 300 && r.top > below;
    return openUp
      ? { left: `${r.left}px`, bottom: `${window.innerHeight - r.top + 2}px` }
      : { left: `${r.left}px`, top: `${r.bottom + 2}px` };
  };
  onMount(() => {
    const reanchor = () => { if (ac()) updateAcRect(); };
    // capture phase so an inner scroller (the feed, the sidebar body) also fires.
    window.addEventListener("scroll", reanchor, true);
    window.addEventListener("resize", reanchor);
    onCleanup(() => {
      window.removeEventListener("scroll", reanchor, true);
      window.removeEventListener("resize", reanchor);
    });
  });

  const closeAc = () => {
    setAc(null);
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
    left.property === right.property;
  const detectEditorTrigger = (value = ref.value, caret = ref.selectionStart): Trigger | null =>
    isCalc() || isAnnot() || !!sheetCell
      ? null
      : detectTrigger(value, caret, propertyValueKey());
  const propertyValueItems = (key: string, query: string): AcItem[] => {
    const values = propertyFacets.find(([candidate]) => candidate === key)?.[1] ?? [];
    const q = query.trim();
    const ranked = values
      .map((value, index) => ({ value, index, score: q ? fuzzyScore(q, value) : 1 }))
      .filter(({ score }) => score > 0)
      .sort((left, right) => right.score - left.score || left.index - right.index)
      .slice(0, 100)
      .map(({ value }) => ({ label: value, propertyValue: value }));
    if (q && !values.some((value) => value.toLowerCase() === q.toLowerCase())) {
      ranked.push({ label: `Create "${q}"`, propertyValue: q });
    }
    return ranked;
  };
  // Page/block/tag/command/code completion is a real transient above its editor
  // and, on mobile, above the drawer. One Escape peels only this popup.
  createEffect(() => {
    if (!ac() || !acItems().length) return;
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
    setAcIndex(0);
    const requestOwner = latestOwner(autocompleteScope, "suggestions", graphOwner(() => sameAcTrigger(ac(), t)));
    if (t.kind === "property-name") {
      const facets = await autocompleteFacets();
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
        ranked.push({ label: `Create "${created}"`, propertyName: created });
      }
      setAcItems(ranked);
      return;
    }
    if (t.kind === "property-value") {
      setAcItems(propertyValueItems(t.property!, t.query));
      return;
    }
    if (t.kind === "code-language") {
      setAcItems(codeLanguageItems(t.query).map((language) => ({
        label: language.label,
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
        return { label: command.label, insert: insertion.insert, caret: insertion.caret };
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
          scored.push({ item: { label: c.label, insert: c.insert, caret: c.caret, action: c.action, taskMarker: c.taskMarker }, s, idx: q ? c.matchTieOrder : c.bareOrder });
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
              item: { label: `Template: ${tp.name}`, templateNodes: tp.blocks },
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
      const result = await readOwned(requestOwner, backend().search(t.query, 20, "block-picker"));
      if (result.kind === "stale") return;
      const items: AcItem[] = [];
      for (const g of result.value) {
        for (const b of g.blocks) {
          items.push({
            label: blockFirstLine(b.raw) || g.page,
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
    const result = await readOwned(requestOwner, cap ? cap.quickSwitch(t.query, 100) : backend().quickSwitch(t.query, 100));
    if (result.kind === "stale") return;
    const pageItem = (name: string): AcItem =>
      t.kind === "page"
        ? { label: name, insert: pageInsert(name), sub: aliasOfLabel(name, navigationName(name)) }
        : { label: `#${name}`, insert: tagInsert(name), sub: aliasOfLabel(name, navigationName(name)) }; // tag context reads "#name"
    const createItem: AcItem =
      t.kind === "page"
        ? { label: `Create "${q}"`, insert: pageInsert(q) }
        : { label: `Create #${q}`, insert: tagInsert(q) };
    setAcItems(orderAcItems(
      result.value.map((page) => ({ name: page.name, item: pageItem(page.name) })),
      { name: q, item: createItem },
      { query: q, policy: linkAutocompletePolicy(), removeAccents: searchRemoveAccents() },
    ));
  };

  // Apply a pure text edit (format toggle / kill motion) to the textarea and
  // restore the resulting selection.
  const applyEdit = (ed: Edit) => {
    commit(ed.text);
    queueMicrotask(() => {
      ref.value = ed.text;
      ref.setSelectionRange(ed.start, ed.end, ed.direction);
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
  const updateSel = () => {
    const selected = ref.selectionStart !== ref.selectionEnd;
    setHasSel(selected);
    if (!selected) setSelectionOverflowOpen(false);
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
  const selectionActionLabel = (action: SelectionAction): JSX.Element => {
    if (action.id === "bold") return <b>{action.label}</b>;
    if (action.id === "italic") return <i>{action.label}</i>;
    if (action.id === "strikethrough") return <s>{action.label}</s>;
    if (action.id === "highlight") return <mark>{action.label}</mark>;
    if (action.id === "link") return (
      <svg viewBox="0 0 24 24" width="13" height="13" aria-hidden="true">
        <path d="M9 15l6-6M10 6l1-1a4 4 0 015.7 5.7l-1 1M14 18l-1 1a4 4 0 01-5.7-5.7l1-1"
          fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
      </svg>
    );
    return action.label;
  };

  // Insert `text` in place of the active trigger and restore the caret. If the
  // completion ends with a closing pair (`]]`/`))`/`}}`) and the same pair
  // sits at or later on this line after the caret (e.g. from a `[[ ]]` autopair
  // or editing inside an existing ref), swallow it so we don't end up with stray
  // ref text or `[[name]]]]`.
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

  // Open the native file picker, copy the chosen file into assets/, and insert
  // its markdown at the caret. Uses the Tauri dialog plugin + import_asset.
  const captureAssetEditorToken = () => captureAssetEditor(ref);
  const assetEditorCurrent = (token: AssetEditorToken) => assetEditorIsCurrent(token, ref, editorMounted);

  // Seed + insert an asset from raw bytes at the caret, then persist to assets/
  // in the background (repointing the link if the backend de-dups the name).
  // Shared by clipboard-image paste and mobile capture (camera / voice memo).
  const insertAssetBytes = async (token: AssetEditorToken, bytes: Uint8Array, origName?: string, captureExt?: string) => {
    const owner = graphOwner(() => assetEditorCurrent(token));
    const candidate = captureExt !== undefined ? captureAssetFileName(captureExt) : assetFileName(origName);
    // Cache key is the bare filename — assetRelPath() strips the `assets/` prefix
    // before loadAssetBlob() (see render/inline.tsx). Seed it so the asset renders
    // instantly, before the disk write lands.
    seedAssetBlob(candidate, bytes);
    let stored: string;
    try {
      // Data before reference: a crash may leave an orphan asset, but can never
      // persist a note that points at bytes which existed only in WebView memory.
      const result = await writeOwned(owner, trackAssetWrite(backend().saveAsset(candidate, bytes, token.binding.backendGeneration)));
      if (result.kind === "stale") { reportStaleAsset(); return; }
      stored = result.value;
    } catch (error) { pushToast(`Couldn’t save to assets/: ${String(error)}`, "error"); return; }
    if (stored !== candidate) seedAssetBlob(stored, bytes);
    const page = pageByName(docNode(props.id)?.page ?? "");
    // Saved images embed inline; other assets, including PDFs, become links.
    const md = assetMarkdown(stored, {
      label: origName,
      pagePath: page?.id,
      format: formatForBlock(props.id),
    });
    // The user may have kept typing while a large capture was being fsynced; use
    // the current selection instead of replaying a stale pre-write offset.
    const start = ref.selectionStart;
    const newRaw = ref.value.slice(0, start) + md + ref.value.slice(ref.selectionEnd);
    commit(newRaw);
    const pos = start + md.length;
    queueMicrotask(() => {
      if (!assetEditorCurrent(token) || ref.value !== newRaw) return;
      ref.value = newRaw;
      ref.setSelectionRange(pos, pos);
      ref.focus();
      autosize();
    });
  };

  const insertStoredAssets = (token: AssetEditorToken, assets: { stored: string; label?: string }[]): boolean => {
    if (!assets.length) return false;
    if (!assetEditorCurrent(token)) {
      reportStaleAsset();
      return false;
    }
    const page = pageByName(docNode(props.id)?.page ?? "");
    const markdown = assets.map(({ stored, label }) => assetMarkdown(stored, {
      label,
      pagePath: page?.id,
      format: formatForBlock(props.id),
    })).join("\n");
    const start = ref.selectionStart;
    const end = ref.selectionEnd;
    const newRaw = ref.value.slice(0, start) + markdown + ref.value.slice(end);
    commit(newRaw);
    const pos = start + markdown.length;
    queueMicrotask(() => {
      if (!assetEditorCurrent(token) || ref.value !== newRaw) return;
      ref.value = newRaw;
      ref.setSelectionRange(pos, pos);
      ref.focus();
      autosize();
    });
    return true;
  };

  const MAX_BYTE_CLIPBOARD_FILE = 64 * 1024 * 1024;
  const MAX_CLIPBOARD_FILES = 32;
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
      ref.value = newRaw;
      ref.setSelectionRange(pos, pos);
      autosize();
    });
  };

  // Transcribed from OG 6e7afa8eb src/main/frontend/handler/paste.cljs:101-107:
  // Markdown recognizes only -, +, *, and ATX headings; Org recognizes stars.
  const plainTextLooksLikeBlocks = (text: string) =>
    pageFmt() === "org"
      ? /^\s*\*+\s+/m.test(text)
      : /^\s*(?:[-+*]|#+)\s+/m.test(text);

  // OG 6e7afa8eb src/main/frontend/handler/paste.cljs:34-47,173-174 splits on
  // two-or-more newlines and trims each whole paragraph before block parsing.
  const segmentedPlainText = (text: string): OutlineNode[] =>
    text.split(/(?:\r?\n){2,}/).map((paragraph) => ({ raw: paragraph.trim(), children: [] }));

  /** Import file-manager paths without materializing their bytes in the WebView.
   * If a platform exposes only browser File objects, save those sequentially so
   * at most one bounded byte buffer/base64 IPC payload is live at a time. */
  const pasteClipboardFiles = async (eventFiles: File[]) => {
    const editorToken = captureAssetEditorToken();
    const owner = graphOwner();
    const toastId = pushToast("Pasting files…", "info");
    let skipped = 0;
    let nativeUnavailable = false;
    const stored: { stored: string; label?: string }[] = [];
    let inserted = false;
    try {
      const nativeResult = await readOwned(ownedWhen(() => editorMounted), backend().clipboardFiles().catch(() => {
        nativeUnavailable = true;
        return { files: [], skipped: 0, truncated: false };
      }));
      if (nativeResult.kind === "stale") return; const native = nativeResult.value;
      if (native.files.length) {
        skipped += native.skipped;
        for (const file of native.files) {
          try {
            const result = await writeOwned(owner, trackAssetWrite(backend().importAsset(file.path, assetFileName(file.name), editorToken.binding.backendGeneration)));
            if (result.kind === "stale") return; stored.push({ stored: result.value, label: file.name });
          } catch (error) {
            pushToast(`Couldn’t import pasted file: ${String(error)}`, "error");
            skipped += 1;
          }
        }
      } else {
        const files = eventFiles.slice(0, MAX_CLIPBOARD_FILES);
        skipped += Math.max(0, eventFiles.length - files.length);
        // Keep the screenshot/image behavior: image-only clipboard payloads use
        // timestamp names and optimistic rendering rather than synthetic names
        // such as Chromium's generic "image.png".
        if (files.length === 1 && files[0].type.startsWith("image/")) {
          const image = files[0];
          if (image.size > MAX_BYTE_CLIPBOARD_FILE) skipped += Math.max(1, native.skipped);
          else {
            try {
              const bytes = new Uint8Array(await image.arrayBuffer());
              if (bytes.length && bytes.length <= MAX_BYTE_CLIPBOARD_FILE) {
                // A Windows bitmap clipboard can appear as one invalid native
                // path plus one valid WebView2 image File. Once the bytes win,
                // suppress that native pseudo-entry's skipped count (GH #78).
                await insertAssetBytes(editorToken, bytes);
              } else skipped += Math.max(1, native.skipped);
            } catch {
              skipped += Math.max(1, native.skipped);
            }
          }
          return;
        }
        skipped += native.skipped;
        for (const file of files) {
          if (file.size > MAX_BYTE_CLIPBOARD_FILE) {
            skipped += 1;
            continue;
          }
          try {
            const bytes = new Uint8Array(await file.arrayBuffer());
            if (!bytes.length) {
              skipped += 1;
              continue;
            }
            const candidate = assetFileName(file.name || undefined);
            const result = await writeOwned(owner, trackAssetWrite(backend().saveAsset(candidate, bytes, editorToken.binding.backendGeneration)));
            if (result.kind === "stale") return; const saved = result.value;
            stored.push({ stored: saved, label: file.name || undefined });
            try {
              seedAssetBlob(saved, bytes);
            } catch {
              // The durable asset + link are authoritative; cache warming is optional.
            }
          } catch (error) {
            pushToast(`Couldn’t save pasted file: ${String(error)}`, "error");
            skipped += 1;
          }
        }
      }
      inserted = insertStoredAssets(editorToken, stored);
    } finally {
      dismissToast(toastId);
      if (!stillBound(editorToken.binding)) return;
      if (inserted) {
        pushToast(`Inserted ${stored.length} file${stored.length === 1 ? "" : "s"}`, "success");
      }
      if (skipped) {
        pushToast(
          `Skipped ${skipped} item${skipped === 1 ? "" : "s"}; folders and byte-only files over 64 MiB aren't pasted`,
          "error"
        );
      } else if (nativeUnavailable && !eventFiles.length) {
        pushToast("Couldn't read copied files; use Upload or drag-and-drop instead", "error");
      }
    }
  };
  // Mobile: take/pick a photo (Android camera plugin) → insert at the caret.
  const capturePhotoCmd = async () => {
    const editorToken = captureAssetEditorToken();
    let res;
    try {
      res = await backend().capturePhoto();
    } catch (err) {
      if (stillBound(editorToken.binding)) pushToast(`Couldn’t capture a photo (${String(err)})`, "error");
      return;
    }
    if (res.status === "ok" && res.path) {
      const candidate = captureAssetFileName(res.ext || "jpg");
      try {
        const stored = await trackAssetWrite(backend().importNativeCapture(res.path, candidate, editorToken.binding.backendGeneration));
        insertStoredAssets(editorToken, [{ stored }]);
      } catch (err) {
        if (stillBound(editorToken.binding)) pushToast(`Couldn’t import the photo (${String(err)})`, "error");
      }
    }
  };

  // Mobile: toggle voice-memo recording. First tap starts (prompts for mic
  // permission); second tap stops and inserts the recorded audio at the caret.
  let mobileRecordingEditorToken: AssetEditorToken | null = null;
  const voiceMemoToggle = async () => {
    if (isRecordingAudio()) {
      const editorToken = mobileRecordingEditorToken;
      mobileRecordingEditorToken = null;
      setRecordingAudio(false);
      let res;
      try {
        res = await backend().stopRecording();
      } catch (err) {
        if (editorToken && stillBound(editorToken.binding)) pushToast(`Couldn’t save the recording (${String(err)})`, "error");
        return;
      }
      if (res.status === "ok" && res.path) {
        const candidate = captureAssetFileName(res.ext || "m4a");
        try {
          if (!editorToken) return;
          const stored = await trackAssetWrite(backend().importNativeCapture(res.path, candidate, editorToken.binding.backendGeneration));
          insertStoredAssets(editorToken, [{ stored }]);
        } catch (err) {
          if (editorToken && stillBound(editorToken.binding)) pushToast(`Couldn’t import the recording (${String(err)})`, "error");
        }
      }
      return;
    }
    const editorToken = captureAssetEditorToken();
    let res;
    try {
      res = await backend().startRecording();
    } catch (err) {
      if (stillBound(editorToken.binding)) pushToast(`Couldn’t start recording (${String(err)})`, "error");
      return;
    }
    if (!stillBound(editorToken.binding)) { if (res.status === "recording") void backend().cancelRecording(); return; }
    if (res.status === "recording") {
      mobileRecordingEditorToken = editorToken;
      setRecordingAudio(true);
      pushToast("Recording… tap the mic again to stop", "info");
    }
  };

  // Desktop voice memo (/record): one process-wide owner reserves the physical
  // recorder before permission, bounds time/bytes, and is cancelled if this editor
  // unmounts. This keeps the microphone reachable and prevents concurrent sessions.
  const desktopRecordingOwner = Symbol(`voice-recording:${props.id}`);
  onCleanup(() => cancelDesktopVoiceRecording(desktopRecordingOwner));
  const desktopVoiceMemoToggle = async () => {
    if (desktopVoiceRecordingActive()) {
      stopDesktopVoiceRecording();
      return;
    }
    const editorToken = captureAssetEditorToken();
    try {
      const status = await startDesktopVoiceRecording(desktopRecordingOwner, {
        complete: async (bytes, mime, limited) => {
          if (limited) pushToast("Recording limit reached; saving the captured audio", "info");
          await insertAssetBytes(editorToken, bytes, undefined, recordingExt(mime));
        },
        error: (message) => pushToast(`Couldn’t save the recording (${message})`, "error"),
      });
      if (status === "busy") {
        pushToast("Another voice recording is already active", "error");
        return;
      }
      pushToast("Recording… run /record again to stop", "info");
    } catch (err) {
      pushToast(`Couldn’t access the microphone (${String(err)})`, "error");
    }
  };
  const uploadAsset = async () => {
    const editorToken = captureAssetEditorToken();
    const owner = graphOwner();
    const picked = await readOwned(ownedWhen(() => editorMounted), backend().pickFile());
    if (picked.kind === "stale" || !picked.value) return; const path = picked.value;
    try {
      // Store with a timestamped name (keeps the original + a sortable insert time).
      const orig = path.split(/[\\/]/).pop() || undefined;
      const saved = await writeOwned(owner, trackAssetWrite(backend().importAsset(path, assetFileName(orig), editorToken.binding.backendGeneration)));
      if (saved.kind === "current") insertStoredAssets(editorToken, [{ stored: saved.value, label: orig }]);
    } catch (error) {
      pushToast(`Couldn’t import asset: ${String(error)}`, "error");
    }
  };

  // `/drawio` creates an editable asset, inserts its reference, then opens the
  // editor. assetRefresh updates the image when Tine regains focus.
  const createDrawioDiagram = async () => {
    const editorToken = captureAssetEditorToken();
    const owner = graphOwner(() => assetEditorCurrent(editorToken));
    const ed = MEDIA_EDITORS.find((e) => e.id === "drawio");
    if (!ed?.blank) return;
    try {
      const bytes = new TextEncoder().encode(ed.blank.contents());
      // Use the unique-stem asset convention (like a camera/mic capture), NOT a
      // fixed `diagram.drawio.svg`: the backend de-dup splits on the LAST dot, so a
      // colliding `diagram.drawio.svg` would become `diagram.drawio_1.svg` — which
      // no longer ends in `.drawio.svg`, dropping the "Edit in draw.io" affordance
      // (GH #38). A unique stem never collides, so the double extension survives.
      const savedResult = await writeOwned(owner, trackAssetWrite(
        backend().saveAsset(captureAssetFileName(ed.blank.ext), bytes, editorToken.binding.backendGeneration)
      ));
      if (savedResult.kind === "stale") return; const saved = savedResult.value;
      insertStoredAssets(editorToken, [{ stored: saved }]);
      const cmd = await resolveMediaEditorCommand(ed);
      if (!owner()) return;
      void writeOwned(owner, backend().editAssetExternal(saved, cmd, editorToken.binding.backendGeneration))
        .catch(() => pushToast("Couldn’t open draw.io", "error"));
      refreshAssetOnReturn(saved);
    } catch (error) {
      pushToast(`Couldn’t create the diagram: ${String(error)}`, "error");
    }
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
      void persistBlockRefTarget(uuid, page, kind, undefined, externalId, () => {
        if (!stillBound(binding) || ac() !== trigger || ref.value !== editorValue) return null;
        const sourcePage = docNode(props.id)?.page;
        if (!sourcePage) return null;
        replaceTrigger(`((${externalId}))`);
        inserted = true;
        return sourcePage;
      }).then((saved) => {
        if (!saved && stillBound(binding) && (inserted || ac() === trigger))
          pushToast("Could not save the block reference. Resolve the page save and try again.", "error");
      }).catch((error) => { if (stillBound(binding)) pushToast(`Could not save the block reference: ${String(error)}`, "error"); });
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
        const liveTrigger = detectTrigger(textarea.value, textarea.selectionStart, propertyValueKey());
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
              pushToast("Plugin result was not inserted because the block changed while it ran.", "info");
            }
            return;
          }
          if (textarea.value !== before) {
            pushToast("Plugin result was not inserted because the block changed while it ran.", "info");
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
        .catch((error) => pushToast(`Plugin slash command failed: ${String(error)}`, "error"));
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
      if (!lastId) { pushToast("Outline is too deep to insert", "error"); return; }
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
          ref.value = next;
          ref.setSelectionRange(next.length, next.length);
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
        // Drop the "/scheduled" trigger text, then open the calendar popup
        // anchored under the editor.
        replaceTrigger("");
        const r = ref.getBoundingClientRect();
        openDatePicker(props.id, item.action, r.left, r.bottom + 4);
        return;
      }
      case "now-time":
        replaceTrigger(timeStamp());
        return;
      case "youtube-timestamp": {
        // OG inserts nothing when no player is registered/ready
        // (youtube.cljs:113-122) — the slash text is still consumed.
        const macro = youtubeTimestampMacroFor(ref);
        replaceTrigger(macro ?? "");
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
        // builder opens its sheet with the field chooser focused (`/query` is one command).
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
        // Drop the trigger text, then open the page-properties panel for the
        // page this block lives on (anchored under the editor).
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

  // Resize the textarea to fit its content. Setting height:auto then reading
  // scrollHeight forces a synchronous layout, so doing it per keystroke thrashes
  // layout; `autosize` coalesces to one resize per animation frame. `resizeNow`
  // is the immediate variant for mount (avoids a one-frame collapsed flash).
  const resizeNow = () => {
    if (!ref || !ref.isConnected) return;
    // Setting height:auto transiently collapses the textarea to measure its
    // content height. When the block is TALLER than the viewport, that collapse
    // makes WebKitGTK scroll-jump the enclosing scroller to keep the caret in
    // view — so every keystroke pinned the caret to the bottom edge of the
    // screen (Martin's report). Preserve the scroller's scrollTop across the
    // measure so autosize stays visually invisible.
    const scroller = nearestScrollableY(ref);
    const prevTop = scroller?.scrollTop;
    ref.style.height = "auto";
    ref.style.height = `${ref.scrollHeight}px`;
    if (scroller && prevTop !== undefined && scroller.scrollTop !== prevTop) {
      scroller.scrollTop = prevTop;
    }
  };
  let autosizeRaf: number | undefined;
  const autosize = () => {
    if (autosizeRaf !== undefined) return; // already scheduled this frame
    autosizeRaf = requestAnimationFrame(() => {
      autosizeRaf = undefined;
      resizeNow();
      pendingScrollAnchor?.restore();
      pendingScrollAnchor = undefined;
    });
  };

  // A `wrap="off"` editor (a code card) mounts with its whole value assigned,
  // which parks the selection at the end; focusing reveals that end and the
  // later setSelectionRange does not scroll back, so a long line opened the
  // editor on blank space hundreds of columns past the code (GH #489). Put the
  // horizontal view where the caret is; one mirror measure, and only for an
  // editor that can scroll horizontally at all, never an ordinary block.
  const revealCaretColumn = (offset: number) => {
    if (!ref || ref.scrollWidth <= ref.clientWidth) return;
    const x = textareaCaretLeft(ref, offset);
    if (x === null) return;
    const margin = 24;
    if (x < ref.scrollLeft + margin) ref.scrollLeft = Math.max(0, x - margin);
    else if (x > ref.scrollLeft + ref.clientWidth - margin) ref.scrollLeft = x - ref.clientWidth + margin;
  };

  const focusNow = () => {
    const historySelection = takeHistoryEditorSelectionFor(props.id, surfaceKey);
    const want = takeCaretFor(props.id);
    ref.focus();
    const v = ref.value;
    if (historySelection) {
      const end = Math.min(historySelection.end, v.length);
      const start = Math.min(historySelection.start, end);
      ref.setSelectionRange(start, end);
      revealCaretColumn(start);
      return;
    }
    if (want !== null && typeof want === "object" && "start" in want) {
      ref.setSelectionRange(want.start, want.end, want.direction);
      revealCaretColumn(want.direction === "backward" ? want.start : want.end);
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
    revealCaretColumn(o);
  };
  onMount(() => {
    const unregisterHistoryTarget = registerHistoryEditorTarget({
      blockId: props.id,
      owner: editingOwner(),
      surface: surfaceKey,
      selection: () => ({ start: ref.selectionStart, end: ref.selectionEnd }),
      viewport: () => ({ editor: ref, scroller: nearestScrollableY(ref) }),
      focused: () => typeof document !== "undefined" && document.activeElement === ref,
    });
    onCleanup(unregisterHistoryTarget);
    // If this block is rendered in several surfaces at once (main pane + sidebar),
    // an unscoped edit (split / keyboard nav) mounts an editor in each. Only the
    // surface that was stamped (the one that had the caret) focuses; the others
    // must NOT steal it. `want === undefined` means "no constraint" → focus as
    // usual (the normal single-surface case — unchanged behaviour).
    const want = focusSurfaceFor(props.id);
    if (want === undefined || want === surfaceKey) {
      focusNow();
      // Clear AFTER this synchronous render flush, so sibling instances mounting
      // in the same flush still see the stamp and stand down.
      queueMicrotask(() => clearFocusSurface(props.id));
    } else {
      // Another surface owns the caret. Safety net against a stale stamp pointing
      // at a surface that doesn't actually render this block: if nothing has taken
      // focus by the next microtask, take it ourselves so the caret never vanishes.
      queueMicrotask(() => {
        if (!ref.isConnected || editingId() !== props.id) return;
        const ae = document.activeElement;
        const taken = ae instanceof HTMLTextAreaElement && ae.classList.contains("block-editor");
        if (!taken) {
          focusNow();
          clearFocusSurface(props.id);
        }
      });
    }
    resizeNow();
    // A split can change the editor's wrapping width after this mount-time
    // measurement. Observe width only so the height write in `resizeNow` cannot
    // feed an observer loop; `autosize` keeps repeated layout changes to one
    // measurement per animation frame.
    if (typeof ResizeObserver !== "undefined") {
      let observedWidth = ref.clientWidth;
      const resizeObserver = new ResizeObserver(() => {
        const width = ref.clientWidth;
        if (width === observedWidth) return;
        observedWidth = width;
        autosize();
      });
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
    setAc(next);
    setAcIndex(0);
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
  const applyFullWidthRefReplace = () => {
    const paired = fullWidthRefReplace(ref.value, ref.selectionStart);
    if (!paired) return false;
    ref.value = paired.value;
    ref.setSelectionRange(paired.caret, paired.caret);
    return true;
  };
  // IMEs replace the textarea's transient composition range many times before
  // committing. Keep that range DOM-local: every setRaw() also dirties the page
  // and starts its save/reparse work. The finalized value has one canonical
  // commit at compositionend instead.
  let compositionActive = false;
  let compositionEndValue: string | null = null;
  const onCompositionStart = () => {
    compositionActive = true;
    compositionEndValue = null;
    clearTimeout(acTimer);
  };
  const onInput = (e: InputEvent) => {
    if (compositionActive || e.isComposing) return;
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
      if (ch === "【") {
        handled = applyFullWidthRefReplace();
      }
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
      if (!handled && typographyMode() === "type") {
        const r = typoTypeReplace(ref.value, ref.selectionStart, ch);
        if (r) {
          ref.value = r.value;
          ref.setSelectionRange(r.caret, r.caret);
        }
      }
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
    applyFullWidthRefReplace();
    compositionEndValue = ref.value;
    commit(ref.value);
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
      // A sibling reorder happens synchronously (a feed move's own sync part)
      // and keeps this textarea. Restore it in the same gesture: waiting a
      // frame lets Android dismiss the IME despite the later focus.
      const move = outlineScope ? moveItem(props.id, dir) : moveBlockFeed(props.id, dir);
      if (ref === movedEditor && movedEditor.isConnected && editingId() === props.id
        && (document.activeElement === movedEditor || document.activeElement === document.body)) restore();
      await move;
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      if (document.activeElement !== ref) restore();
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
    const { raw: newRaw, delta } = cycleMarkerSmart(ref.value, workflow());
    commit(newRaw);
    const pos = Math.max(0, start + delta);
    queueMicrotask(() => {
      ref.value = newRaw;
      ref.setSelectionRange(pos, pos);
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
  const openScheduledDatePicker = () => {
    const r = ref.getBoundingClientRect();
    openDatePicker(props.id, "scheduled", r.left, r.bottom + 4);
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
    "editor/bold": (e) => { e.preventDefault(); applyInlineFormat("bold"); return true; },
    "editor/italics": (e) => { e.preventDefault(); applyInlineFormat("italic"); return true; },
    "editor/strike-through": (e) => { e.preventDefault(); applyInlineFormat("strikethrough"); return true; },
    "editor/highlight": (e) => { e.preventDefault(); applyInlineFormat("highlight"); return true; },
    "editor/insert-link": (e) => { e.preventDefault(); applyEdit(insertLink(ref.value, ref.selectionStart, ref.selectionEnd, pageFmt())); return true; },
    // GH #279: embed twin of Mod+C; with a text selection decline so the platform copy runs.
    "editor/copy-embed": (e) => { if (ref.selectionStart !== ref.selectionEnd) return false; e.preventDefault(); commit(ref.value); void copyBlockLink(props.id, "embed"); return true; },
    "editor/clear-block": (e) => { e.preventDefault(); applyEdit({ text: "", start: 0, end: 0 }); return true; },
    "editor/kill-line-before": (e) => { e.preventDefault(); applyEdit(killLineBefore(ref.value, ref.selectionStart)); return true; },
    "editor/kill-line-after": (e) => { e.preventDefault(); applyEdit(killLineAfter(ref.value, ref.selectionStart)); return true; },
    "editor/backward-kill-word": (e) => { e.preventDefault(); applyEdit(killWordBackward(ref.value, ref.selectionStart)); return true; },
    "editor/forward-kill-word": (e) => { e.preventDefault(); applyEdit(killWordForward(ref.value, ref.selectionStart)); return true; },
    "editor/backward-word": (e) => { e.preventDefault(); moveCaret(wordBackward(ref.value, ref.selectionStart)); return true; },
    "editor/forward-word": (e) => { e.preventDefault(); moveCaret(wordForward(ref.value, ref.selectionStart)); return true; },
    "editor/move-block-up": (e) => moveBlockCmd(e, -1),
    "editor/move-block-down": (e) => moveBlockCmd(e, 1),
    "editor/collapse": (e) => { e.preventDefault(); setCollapsed(props.id, true); return true; },
    "editor/expand": (e) => { e.preventDefault(); setCollapsed(props.id, false); return true; },
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
      // On an in-block list line, Tab nests the LIST ITEM (intra-block), not the block.
      const ll = listLineAt(ref.value, ref.selectionStart, pageFmt());
      if (ll) { nudgeListItem(ll, +2); return true; }
      if (outlineScope?.roots.includes(props.id)) return true;
      const selection = { start: ref.selectionStart, end: ref.selectionEnd, direction: ref.selectionDirection };
      commit(ref.value);
      if (indentBlock(props.id, selection, editSurface()) === false) pushToast("Outline is too deep to indent", "error");
      return true;
    },
    "editor/outdent": (e) => {
      e.preventDefault();
      const ll = listLineAt(ref.value, ref.selectionStart, pageFmt());
      if (ll && ll.indent.length > 0) { nudgeListItem(ll, -2); return true; }
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
    }
  };
  let unregisterFocusedEditorBridge: (() => void) | undefined;
  const registerFocusedEditorBridge = () => {
    unregisterFocusedEditorBridge?.();
    unregisterFocusedEditorBridge = registerFocusedEditorCommandBridge({
      blockId: props.id,
      dispatch: dispatchMobileEditorCommand,
      blur: () => ref.blur(),
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
    splitProps(docNode(id)?.raw ?? "", isSheetCellHidden).visible.length;
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

    const start = ref.selectionStart;
    const end = ref.selectionEnd;
    const raw = ref.value;

    // Ctrl/Cmd+Shift+V is Logseq's universal raw-paste gesture.
    // ClipboardEvent does not expose modifier keys, so remember the preceding
    // keydown briefly and consume it in onPaste. Do not preventDefault: the
    // platform still has to perform the native clipboard read and dispatch paste.
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "v") {
      clearPasteRaw();
      pasteRaw = true;
      const token = ++pasteRawToken;
      pasteRawTimer = window.setTimeout(() => {
        if (pasteRawToken === token) clearPasteRaw();
      }, 1_000);
      return;
    }
    if (pasteRaw) clearPasteRaw();

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
      e.key.toLowerCase() === "c" &&
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
    if (cap) {
      // File the capture via the configurable `editor/quick-capture-file`
      // shortcut (default mod+shift+enter). `cmd` is already resolved above; it's
      // not in runEditorCmd, so it fell through to here. Remappable in Settings →
      // Keyboard shortcuts (and the capture window syncs the user's binding).
      if (cmd === "editor/quick-capture-file") {
        e.preventDefault();
        commit(raw);
        cap.submit();
        return;
      }
      // "Enter files" mode: a popup-closed plain Enter files. In "new block" mode
      // it falls through to the normal split-into-a-new-block handling below.
      if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey && cap.enterFiles()) {
        e.preventDefault();
        commit(raw);
        cap.submit();
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        cap.cancel();
        return;
      }
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
      const inFence = !isAnnot() && caretInFence(raw, start);
      // GH #278: a multi-line `$$ … $$` environment behaves like a fence for
      // Enter. See caretInDisplayMath — a deliberate divergence from OG.
      const inMath = !isAnnot() && !inFence && caretInDisplayMath(raw, start);
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
        const trimmed = kind === "code" ? codeBodyExitTrim(raw, start) : multilineExitTrim(raw, start, kind);
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
      if (!isAnnot() && (inFence || inMath || caretOnOpeningFence(raw, start))) {
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
      const ll = !isAnnot() ? listLineAt(raw, start, pageFmt()) : null;
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
      const ll = listLineAt(raw, start, pageFmt());
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
        if (mergeWithPrev(props.id, outlineScope, editSurface())) {
          e.preventDefault();
          return;
        }
        const n = docNode(props.id);
        const next = nextVisible(props.id, outlineScope);
        if (n && splitProps(n.raw, hideFn(), pageFmt()).visible.trim() === "" && n.children.length === 0 && next && docNode(next)?.page === n.page) {
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
      const next = nextVisible(props.id, outlineScope);
      if (next) {
        const nextRaw = docNode(next)?.raw ?? "";
        if (isAnnotationBlock(nextRaw) || calcSource(nextRaw) !== null) return;
        commit(raw);
        if (mergeWithNext(props.id, outlineScope, editSurface())) {
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
        const prev = prevVisible(props.id, outlineScope);
        if (prev) {
          e.preventDefault();
          // A number caret clamps to the new editor's full text length at mount.
          startEditing(prev, Number.MAX_SAFE_INTEGER, null, editSurface());
        }
      }
    } else if (e.key === "ArrowRight" && !e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey) {
      // GH #213: at the very end, move into the START of the next visible editor.
      if (start === end && start === raw.length) {
        const next = nextVisible(props.id, outlineScope);
        if (next) {
          e.preventDefault();
          startEditing(next, 0, null, editSurface());
        }
      }
    } else if (e.key === "ArrowUp" && !e.shiftKey) {
      // Leave for the previous block only from the FIRST visual row; otherwise
      // let the textarea move the caret up one wrapped line. (A long single line
      // has no `\n` but still wraps — the source-`\n` test alone wrongly jumped
      // to the parent from the second visual row.)
      const before = raw.slice(0, start);
      if (!before.includes("\n") && caretAtFirstRow(ref, start)) {
        let prev = prevVisible(props.id, outlineScope);
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
          startEditing(prev, { col: start - (before.lastIndexOf("\n") + 1), edge: "last" }, null, exitingEmbed ? null : editSurface());
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
        const next = nextVisible(props.id, outlineScope);
        if (next) {
          e.preventDefault();
          startEditing(next, { col, edge: "first" }, null, editSurface());
        } else {
          // No next LOADED block. In the journal feed, pull in the next day so
          // Down-arrow keeps going past the loaded window (previously only a
          // mouse-wheel scroll grew the feed). Non-feed pages resolve to null → a
          // harmless no-op. Async: flush first, then step into the new day.
          if (!outlineScope) {
            e.preventDefault();
            commit(raw);
            void nextVisibleOrExtend(props.id).then((n) =>
              n && startEditing(n, { col, edge: "first" }, null, editSurface())
            );
          }
        }
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      selectBlock(props.id, outlineScope); // exit editing into block-selection mode
    }
  };

  const onBlur = () => {
    clearPasteRaw();
    unregisterFocusedEditor();
    if (sheetCanceling) return;
    // A block-move reorder blurs us momentarily — stay in edit mode (the move
    // handler refocuses and restores the caret). Commit as-is, don't normalize.
    if (isBlockMoving()) {
      commit(ref.value);
      return;
    }
    // Ctrl+F moves focus into Tine's find bar, but the block should remain in
    // edit mode so Escape can restore the caret instead of remounting rendered
    // content underneath the user.
    if (inPageFindPreservesEditorBlur()) {
      commit(ref.value);
      savedSel = { start: ref.selectionStart, end: ref.selectionEnd };
      return;
    }
    // The whole window lost focus (switched to another app/window): stay in edit
    // mode and remember the caret so onWindowFocus can resume exactly here. Commit
    // as-is — we're still editing, not exiting.
    if (!document.hasFocus()) {
      commit(ref.value);
      savedSel = { start: ref.selectionStart, end: ref.selectionEnd };
      return;
    }
    // A real exit (clicking elsewhere, Escape, Enter→new block): move any
    // SCHEDULED/DEADLINE planning line to its canonical position (OG layout) as we
    // commit — type-anywhere-while-editing, normalize-on-exit (M1c). The editor is
    // closing, so there is no caret to preserve.
    const calcExit = isCalc();
    commit(calcExit ? ref.value : normalizePlanning(ref.value, pageFmt()), calcExit ? { calc: true } : undefined);
    finishPageHeaderEdit(props.id);
    // Only clear if no other block grabbed editing focus.
    if (editingId() === props.id) endEdit("blur");
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
  onMount(() => window.addEventListener("focus", onWindowFocus));
  onCleanup(() => window.removeEventListener("focus", onWindowFocus));

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
    const syntaxSensitive = sheetCell || isCalc() || codeShown() !== null || caretInFence(ref.value, start)
      || caretOnOpeningFence(ref.value, start) || caretInDisplayMath(ref.value, start);
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
    const htmlNodes = syntaxSensitive ? null : structuredHtmlOutline(html, text, pageFmt());
    if (htmlNodes) {
      e.preventDefault();
      insertPastedOutline(htmlNodes, "structured-paste");
      return;
    }
    if (text.includes("\n")) {
      e.preventDefault();
      if (text.length > OUTLINE_MAX_SOURCE_CHARS) { pushToast("Pasted text is too large", "error"); return; }
      if (syntaxSensitive) {
        pasteLiteralText(text);
        return;
      }
      if (!plainTextLooksLikeBlocks(text) && !/(?:\r?\n){2,}/.test(text)) {
        pasteLiteralText(text);
        return;
      }
      const nodes = plainTextLooksLikeBlocks(text)
        ? parseOutline(text)
        : segmentedPlainText(text);
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
        !caretInFence(ref.value, start) &&
        !caretOnOpeningFence(ref.value, start)
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
    const looksImage = (e.clipboardData?.types ?? []).some(
      (t) => t.startsWith("image/") || t === "Files"
    );
    const toastId = looksImage ? pushToast("Pasting image…", "info") : 0;
    const editorToken = captureAssetEditorToken();
    const owner = graphOwner(() => assetEditorCurrent(editorToken));
    void (async () => {
      let bytes: Uint8Array | null = null;
      try {
        const result = await readOwned(owner, backend().readClipboardImage());
        if (result.kind === "stale") return; bytes = result.value;
      } finally {
        if (toastId) dismissToast(toastId);
      }
      if (!bytes) return;
      await insertAssetBytes(editorToken, bytes);
    })();
  };
  function insertPastedOutline(nodes: OutlineNode[], tag: string, asChildren = false) {
    const current = docNode(props.id);
    if (!current) return;
    if (!outlineFits(props.id, nodes, asChildren ? 1 : 0)) { pushToast("Pasted outline is too deep", "error"); return; }
    const rawAtPaste = ref.value;
    const pageAtPaste = current.page;
    const insert = (prepared: OutlineNode[] | null) => {
      if (!prepared?.length || !ref.isConnected || ref.value !== rawAtPaste || docNode(props.id)?.page !== pageAtPaste) return;
      if (asChildren) {
        const inserted = withUndoUnit(tag, [pageAtPaste], () => {
          commit(rawAtPaste);
          return insertOutlineChildren(props.id, prepared) ?? false;
        });
        if (!inserted) pushToast("Pasted outline was refused", "error");
        return;
      }
      const wasEmpty = rawAtPaste.trim() === "" && docNode(props.id).children.length === 0;
      const lastId = withUndoUnit(tag, [pageAtPaste], () => {
        commit(rawAtPaste);
        return (wasEmpty
          ? replaceEmptyBlockWithOutline(props.id, prepared)
          : insertOutlineAfter(props.id, prepared)) ?? false;
      });
      if (!lastId) { pushToast("Pasted outline was refused", "error"); return; }
      if (docNode(lastId)) startEditing(lastId, docNode(lastId).raw.length);
    };
    const prepared = sanitizeOutlineIdsForPaste(props.id, nodes);
    if (prepared instanceof Promise) void prepared.then(insert);
    else insert(prepared);
  }

  return (
    <div class="editor-wrap" classList={{ "calc-wrap": isCalc() }}>
      <Show when={isCalc()}>
        <div class="calc-gutter" aria-hidden="true">
          <For each={calcRows()}>{(_, i) => <div class="calc-lineno">{i() + 1}</div>}</For>
        </div>
      </Show>
      <textarea
        ref={ref}
        class="block-editor"
        classList={{ [`h${editorHeadingLevel()}`]: editorHeadingLevel() != null, "code-edit": codeEditing() }}
        wrap={codeEditing() ? "off" : "soft"}
        spellcheck={spellcheckEnabled()}
        value={isCalc() ? (calcLive() ?? "") : (codeShown()?.body ?? editorValue())}
        placeholder={cap?.bulletHint?.()}
        onInput={onInput}
        onCompositionStart={onCompositionStart}
        onCompositionEnd={onCompositionEnd}
        onKeyDown={onKeyDown}
        onKeyUp={(e) => {
          if (e.key.toLowerCase() === "v") clearPasteRaw();
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
      <Show when={isCalc()}>
        <div class="calc-results">
          <For each={calcRows()}>
            {(r) => (
              <div class="calc-out" classList={{ "calc-error": !!r.error }}>
                {r.output ?? ""}
                <Show when={r.output !== null && !r.error}>
                  <CopyButton text={r.output!} title="Copy result" class="calc-copy" />
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
      <Show when={hasSel()}>
        <div class="sel-toolbar" classList={{ "sel-toolbar-mobile": isMobilePlatform }} data-mobile-selection-toolbar={isMobilePlatform ? "" : undefined} onMouseDown={(e) => e.preventDefault()}>
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
            title="More formatting"
            aria-label="More formatting"
            aria-haspopup="menu"
            aria-expanded={selectionOverflowOpen()}
            onClick={() => setSelectionOverflowOpen((open) => !open)}
          >…</button>
          <Show when={selectionOverflowOpen()}>
            <div ref={selectionOverflowRef} class="sel-toolbar-overflow" role="menu" aria-label="More formatting">
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
      <Show when={ac() && acItems().length > 0 && acRect()}>
        {/* Portaled to <body> + position:fixed so the right sidebar's overflow
            (or any clipping ancestor) can't cut the dropdown off.
            data-lenis-prevent: with smooth scrolling on, scroll it natively. */}
        <Portal>
          <div class="autocomplete" ref={acListRef} data-lenis-prevent style={acStyle()}>
            <For each={acItems()}>
              {(item, i) => (
                <div
                  class="ac-item"
                  classList={{ active: i() === acIndex() }}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    selectAc(item);
                  }}
                >
                  <span class="ac-label">{item.label}</span>
                  <Show when={item.sub}>
                    <span class="ac-sub">{item.sub}</span>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Portal>
      </Show>
    </div>
  );
}

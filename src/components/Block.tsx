import { Show, Switch, Match, For, createMemo, createSignal, createContext, useContext, createUniqueId, createEffect, onMount, onCleanup, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { backend } from "../backend";
import {
  detectTrigger,
  applyCompletion,
  autoPairEdit,
  pageInsert,
  tagInsert,
  orderAcItems,
  COMMANDS,
  commandScore,
  fuzzyScore,
  type Trigger,
} from "../editor/autocomplete";
import { autoPairInsertOnInput, wrapSelectionEdit, doubleRefKind, backspacePairEdit, SELECTION_WRAP } from "../editor/autopair";
import { typoTypeReplace } from "../render/typography";
import { linkFirstMatch } from "../editor/linkDefault";
import { spellcheckEnabled } from "../spellcheckSettings";
import {
  doc,
  pageByName,
  setRaw,
  setBlockProperty,
  splitBlock,
  indentBlock,
  outdentBlock,
  mergeWithPrev,
  toggleCollapse,
  setCollapsed,
  prevVisible,
  nextVisible,
  nextVisibleOrExtend,
  insertEmptyChildBlock,
  insertOutlineAfter,
  deleteBlock,
  moveBlock,
  moveBlockFeed,
  selectBlock,
  extendSelectionTo,
  clearSelection,
  moveSelection,
  isSelected,
  ensureBlockId,
  persistentBlockRef,
  persistBlockRefTarget,
  isBlockMoving,
  setBlockMoving,
  orderedListMarker,
  withUndoUnit,
  blockIsGridView,
} from "../store";
import {
  clearFocusSurface,
  editingId,
  editingOwner,
  endEdit,
  focusSurfaceFor,
  noteSurfaceFocused,
  startEditing,
  takeCaretFor,
} from "../editorController";
import { parseOutline } from "../editor/outline";
import {
  toggleWrap,
  insertLink,
  wrapLink,
  isPasteableUrl,
  killLineBefore,
  killLineAfter,
  wordForward,
  wordBackward,
  killWordForward,
  killWordBackward,
  setPriority,
  type Edit,
} from "../editor/format";
import { isRenderHiddenProp, isPropertyLine } from "../render/block";
import { facetsOf } from "../render/facets";
import { AstBody } from "../render/body";
import { InlineText } from "../render/inline";
import { editorOffsetFromRenderedRange } from "../render/spans";
import {
  assetMarkdown,
  assetFileName,
  insertedAssetMarkdownTarget,
  replaceInsertedAssetMarkdown,
} from "../media";
import { calcSource, wrapCalc, evalCalc } from "../editor/calc";
import { QueryMacro, EmbedMacro } from "./Macro";
import { workflow, zoomInto, openContextMenu, openDatePicker, openBlockInSidebar, graphMeta, dataRev, setQueryBuilderAutoOpen, openPageProps, pushToast, dismissToast, autoPairing, typographyMode, timetrackingEnabled, logbookWithSecondSupport } from "../ui";
import { seedAssetBlob } from "../assetCache";
import { openPageInNewTab } from "../router";
import { blockRefCount } from "../blockRefCounts";
import { BlockReferences } from "./BlockReferences";
import { editorCommandFor } from "../keybindings";
import { cycleMarkerSmart, toggleTaskDone } from "../editor/repeat";
import { taskCheckboxState } from "../markers";
import { applyTemplateVars } from "../editor/templateVars";
import { caretAtFirstRow, caretAtLastRow } from "../editor/caretRows";
import { splitProps, joinProps, isBuiltinHidden, isSheetCellHidden, hideAll, caretInFence } from "../editor/properties";
import { normalizePlanning } from "../editor/planning";
import { isAnnotationBlock, annotationInfo } from "../editor/annotation";
import { AnnotationBody } from "./AnnotationBody";
import { logbookInfo, type LogbookInfo } from "../logbook";
import { inPageFindPreservesEditorBlur } from "../inpageFind";
import { registerFocusedEditorCommandBridge, type MobileEditorCommandId } from "../editorCommandBridge";
import { isRecordingAudio, setRecordingAudio, base64ToBytes } from "../mediaCapture";
import { sheetConfig } from "../sheet/config";
import { SheetCellContext } from "../sheet/context";
import { appendSheetCellChild } from "../sheet/mutations";
import { cellBlockId, cellOwner, selectCellAfterEdit, moveCellAfterEdit, selectTopRowSeamAfterEdit } from "../sheet/selection";
import { forbidsEditEntry } from "../editor/editTargets";
import { SheetGrid } from "./SheetGrid";
import { SheetTable } from "./SheetTable";
import { SheetBoard } from "./SheetBoard";
import { blockBackgroundColor } from "../blockColors";
import { SheetContainer } from "./SheetContainer";

type SheetSlashView = "grid" | "table" | "board";

export function applySheetViewSlashAction(id: string, view: SheetSlashView): string | null {
  const node = doc.byId[id];
  if (!node) return null;
  let seededCellId: string | null = null;
  withUndoUnit(`sheet:view:${view}`, [node.page], () => {
    const shouldSeedGrid = view === "grid" && (doc.byId[id]?.children.length ?? 0) === 0;
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

// Detect a block whose entire body is a single {{query}} / {{embed}} macro.
function detectMacro(raw: string): { kind: "query" | "embed"; inner: string } | null {
  // The macro is the block's visible body — strip property lines (the shared line
  // recognizer) so a `{{query}}\nid:: …` block still matches. Cheap: no parse.
  const text = raw.split("\n").filter((l) => !isPropertyLine(l)).join("\n").trim();
  const m = /^\{\{(query|embed)\b([\s\S]*)\}\}$/.exec(text);
  if (!m) return null;
  return { kind: m[1] as "query" | "embed", inner: `${m[1]}${m[2]}` };
}

// Any body LINE that is exactly a {{query …}} macro (same recognizer as
// detectMacro, applied per line — the macro may share the block with a heading
// or other text). Not fence-aware; a fenced {{query}} inside a block that ALSO
// declares tine.view:: table/board is not a real case.
function bodyContainsQueryMacro(raw: string): boolean {
  return raw.split("\n").some((l) => /^\{\{query\b[\s\S]*\}\}$/.test(l.trim()));
}

// (Rendered-property hidden set lives in render/block.ts as RENDER_HIDDEN_PROPS /
// isRenderHiddenProp, shared with body.tsx's renderProps.)

// Pointer-based drag reorder (HTML5 DnD is unreliable in WebKitGTK).
const [dragId, setDragId] = createSignal<string | null>(null);
const [dropInd, setDropInd] = createSignal<{ id: string; before: boolean } | null>(null);
let dragMoved = false;

function siblingIndex(id: string): number {
  const n = doc.byId[id];
  if (!n) return -1;
  const sibs =
    n.parent === null
      ? doc.pages.find((p) => p.name === n.page)?.roots ?? []
      : doc.byId[n.parent].children;
  return sibs.indexOf(id);
}

function beginDrag(id: string, e: MouseEvent) {
  const startX = e.clientX;
  const startY = e.clientY;
  dragMoved = false;
  const onMove = (ev: MouseEvent) => {
    if (!dragMoved && Math.hypot(ev.clientX - startX, ev.clientY - startY) < 4) return;
    if (!dragMoved) {
      dragMoved = true;
      setDragId(id);
      endEdit("drag-start");
    }
    const el = (document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null)?.closest(
      ".ls-block"
    ) as HTMLElement | null;
    const tid = el?.dataset.blockId;
    if (tid && tid !== id) {
      const main = el!.querySelector(".block-main")!.getBoundingClientRect();
      setDropInd({ id: tid, before: ev.clientY < main.top + main.height / 2 });
    } else {
      setDropInd(null);
    }
  };
  const onUp = () => {
    document.removeEventListener("mousemove", onMove);
    document.removeEventListener("mouseup", onUp);
    const ind = dropInd();
    if (dragMoved && ind && doc.byId[ind.id]) {
      const tgt = doc.byId[ind.id];
      // can't drop onto own descendant
      let p: string | null = ind.id;
      let ok = true;
      while (p !== null) {
        if (p === id) {
          ok = false;
          break;
        }
        p = doc.byId[p].parent;
      }
      // Pass the target's page so a root-to-root drop across pages (e.g. between
      // journal days) lands on the page it was dropped onto, not the source page.
      if (ok) void moveBlock(id, tgt.parent, siblingIndex(ind.id) + (ind.before ? 0 : 1), tgt.page);
    }
    setDragId(null);
    setDropInd(null);
    setTimeout(() => (dragMoved = false), 0);
  };
  document.addEventListener("mousemove", onMove);
  document.addEventListener("mouseup", onUp);
}

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
}
export const CaptureCtx = createContext<CaptureApi | null>(null);

// Identifies the editing SURFACE a block is rendered in (the main pane vs a
// specific right-sidebar item). Defaults to "main"; RightSidebar overrides it per
// item. Used to arbitrate edit-focus when one block uuid renders in several
// surfaces at once (see startEditing's surface stamping).
export const SurfaceContext = createContext<string>("main");

export function Block(props: { id: string; hideRefCount?: boolean }): JSX.Element {
  const node = () => doc.byId[props.id];
  // Unique per rendered instance, so when one block uuid appears in several
  // surfaces only the instance that was clicked mounts the editor (the rest stay
  // rendered and reflect edits live). null owner = unscoped (keyboard nav).
  const instanceId = createUniqueId();
  // This block's edit "surface": the main pane, a sidebar item, or a secondary
  // "ref:…" reference view (agenda / {{query}} / {{embed}} / linked+block refs —
  // all keyed by LiveRefGroup). Drives which instance shows the editor.
  const surfaceKey = useContext(SurfaceContext);
  const editing = () => {
    if (editingId() !== props.id) return false;
    const owner = editingOwner();
    // Scoped (a click): only the exact instance that was clicked edits; every other
    // instance of this uuid stays rendered and reflects the edit live.
    if (owner !== null) return owner === instanceId;
    // Unscoped (keyboard nav / split): edit in the PRIMARY surface where the caret
    // already was. A block that also appears in a secondary "ref:" surface (e.g. the
    // journal agenda re-lists today's scheduled/deadline bullets) must stay RENDERED
    // there — arrowing into the real bullet must not flip the agenda copy into an
    // editor. (Clicking a ref/agenda copy still edits it in place, via the branch
    // above.) Matches the sidebar rule: edit where you're editing, render elsewhere.
    return !surfaceKey.startsWith("ref:");
  };
  const hasChildren = () => node().children.length > 0;
  const collapsed = () => node().collapsed;
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
  const headingLevel = createMemo(() => blockFacets()?.headingLevel ?? null);
  const editorVisibleValue = createMemo(() => {
    const n = node();
    return n ? splitProps(n.raw, isBuiltinHidden).visible : "";
  });
  const editorIsUniline = createMemo(() => !editorVisibleValue().includes("\n"));
  // Block-level "linked references" panel toggled by the reference-count badge.
  const [showRefs, setShowRefs] = createSignal(false);
  // Ordered-list label for THIS block's own bullet (OG numbers the block itself,
  // not its children); null for a normal bullet.
  const orderMarker = () => orderedListMarker(props.id);
  // An org page Tine can't round-trip is shown but NOT editable (Tine must never
  // rewrite it). Clicking a block doesn't enter the editor on such a page.
  const readOnly = () => pageByName(node().page)?.readOnly ?? false;

  return (
    <div class="ls-block" classList={{ collapsed: collapsed() }} data-block-id={props.id}>
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
          e.preventDefault();
          openContextMenu(e.clientX, e.clientY, props.id);
        }}
      >
        <div class="block-controls">
          <span
            class="collapse-toggle"
            classList={{ "has-children": hasChildren() }}
            onClick={() => toggleCollapse(props.id)}
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
              if (e.button === 0) beginDrag(props.id, e);
            }}
            onClick={(e) => {
              e.stopPropagation();
              if (dragMoved) return; // was a drag, not a click
              if (e.shiftKey) openBlockInSidebar(persistentBlockRef(props.id));
              else zoomInto(props.id);
            }}
            onAuxClick={(e) => {
              if (e.button !== 1) return; // middle-click → open the zoom in a new tab
              e.preventDefault();
              e.stopPropagation();
              const ref = persistentBlockRef(props.id); // writes id:: so the tab survives a restart
              openPageInNewTab(ref.page, ref.pageKind, ref.uuid);
            }}
          >
            <Show when={orderMarker()} fallback={<span class="bullet" />}>
              <span class="bullet-order">{orderMarker()}.</span>
            </Show>
          </span>
        </div>

        <div
          class="block-content-wrapper"
          classList={{ "read-only": readOnly() }}
          onMouseDown={(e) => {
            // Mousedown anywhere in the row (not on a link/chip) arms the same
            // click-or-drag gesture as block content, with an end-of-block caret
            // (the row padding has no text to map). Read-only org pages don't edit.
            if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return;
            if (!editing() && !readOnly() && !forbidsEditEntry(e))
              beginEditGesture(e, props.id, doc.byId[props.id].raw.length, instanceId);
          }}
        >
          <Show
            when={editing()}
            fallback={
              <Rendered
                id={props.id}
                owner={instanceId}
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
                        if (e.shiftKey) openBlockInSidebar(persistentBlockRef(props.id));
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

      <Show when={!collapsed() && (hasChildren() || sheet().view === "grid" || sheet().view === "table" || sheet().view === "board")}>
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
              <div class="block-children-left-border" />
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

// --- Click / drag gesture on rendered block content -------------------------
//
// The caret offset is captured at MOUSEDOWN (before the previously-edited
// block's blur reflows the layout — the coordinates are only valid then), but
// editing starts at MOUSEUP and only for a CLICK (pointer moved < threshold).
// A drag instead selects: within the origin block it is the browser's native
// text selection of the RENDERED text (copy gives the glyphs you see); the
// moment it crosses into another block it escalates to Tine's block selection
// (muscle memory from OG — but deterministic: the escalation rule is purely
// "did the pointer enter a different block", never timing).
//
// Deliberately NOT OG's mousedown-instant-edit: that races the native
// selection against the DOM swap (the inconsistency Martin observed in OG).
const DRAG_THRESHOLD_PX = 4;
const SHEET_CELL_BLOCKED_EDITOR_COMMANDS = new Set([
  "editor/indent",
  "editor/outdent",
  "editor/move-block-up",
  "editor/move-block-down",
  "editor/select-block-up",
  "editor/select-block-down",
]);

interface EditGesture {
  blockId: string;
  offset: number;
  owner: string | null;
  startX: number;
  startY: number;
  escalated: boolean;
}

function blockIdAtPoint(x: number, y: number): string | null {
  const el = document.elementFromPoint(x, y);
  const row = el?.closest?.(".ls-block");
  return row?.getAttribute("data-block-id") ?? null;
}

/** Arm a click-or-drag gesture from a rendered-content mousedown. Document-level
 *  listeners resolve it, so post-blur layout shifts can't misroute the mouseup. */
function beginEditGesture(e: MouseEvent, blockId: string, offset: number, owner: string | null): void {
  clearSelection(); // a plain gesture replaces any active block selection (shift-click returns before this)
  const g: EditGesture = { blockId, offset, owner, startX: e.clientX, startY: e.clientY, escalated: false };
  const onMove = (ev: MouseEvent) => {
    const moved =
      Math.abs(ev.clientX - g.startX) > DRAG_THRESHOLD_PX || Math.abs(ev.clientY - g.startY) > DRAG_THRESHOLD_PX;
    if (!moved) return;
    const over = blockIdAtPoint(ev.clientX, ev.clientY);
    if (g.escalated) {
      if (over) extendSelectionTo(over);
      return;
    }
    if (over && over !== g.blockId) {
      // Crossed into another block: escalate to block selection for the rest of
      // the gesture (never de-escalate — flipping modes mid-drag is jarring).
      g.escalated = true;
      window.getSelection()?.removeAllRanges();
      selectBlock(g.blockId);
      extendSelectionTo(over);
    }
  };
  const onUp = (ev: MouseEvent) => {
    document.removeEventListener("mousemove", onMove, true);
    document.removeEventListener("mouseup", onUp, true);
    if (g.escalated) return; // block selection stands
    const moved =
      Math.abs(ev.clientX - g.startX) > DRAG_THRESHOLD_PX || Math.abs(ev.clientY - g.startY) > DRAG_THRESHOLD_PX;
    if (moved) return; // an in-block text selection (or a stray drag) — not a click
    startEditing(g.blockId, g.offset, g.owner);
  };
  document.addEventListener("mousemove", onMove, true);
  document.addEventListener("mouseup", onUp, true);
}

function Rendered(props: { id: string; owner?: string; trailing?: JSX.Element }): JSX.Element {
  const node = () => doc.byId[props.id];
  const fmt = () => pageByName(node().page)?.format ?? "md";
  // Header facets (marker/priority/heading/scheduled/deadline/properties) off the
  // ONE lsdoc parse — read from the cache the store seeded from the backend DTO (no
  // parse on load), recomputed from a single wasm parse only for the edited block.
  const facets = createMemo(() => facetsOf(node().raw, fmt()));
  const clock = createMemo((): LogbookInfo | null => {
    if (!timetrackingEnabled()) return null;
    const marker = facets().marker;
    if (marker !== "DONE" && marker !== "TODO" && marker !== "LATER") return null;
    const info = logbookInfo(node().raw);
    return info.seconds > 0 ? info : null;
  });
  const readOnly = () => pageByName(node().page)?.readOnly ?? false;

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
  const clickOffset = (e: MouseEvent): number | null => {
    if (!contentRef) return null;
    const d = document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
    const range = d.caretRangeFromPoint?.(e.clientX, e.clientY);
    if (!range) return null;
    return editorOffsetFromRenderedRange(contentRef, range, node().raw, isBuiltinHidden);
  };
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
    beginEditGesture(e, props.id, clickOffset(e) ?? node().raw.length, props.owner ?? null);
  };

  const displayProps = () => {
    const extra = graphMeta()?.block_hidden_properties ?? [];
    return facets().properties.filter(([k]) => !isRenderHiddenProp(k, extra));
  };
  const bgColor = () => {
    return blockBackgroundColor(facets().properties);
  };

  const body = (
    <Show when={annotation()} fallback={<AstBody raw={node().raw} blockId={props.id} format={fmt()} headingLevel={facets().headingLevel} />}>
      <AnnotationBody
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
              <QueryMacro body={macro()!.inner} blockId={props.id} />
            </Match>
            <Match when={macro()!.kind === "embed"}>
              <EmbedMacro body={macro()!.inner} />
            </Match>
          </Switch>
        </div>
      }
    >
    <div
      ref={contentRef}
      class="block-content"
      classList={{ done: facets().done, "has-bg": !!bgColor(), [`heading h${facets().headingLevel ?? ""}`]: facets().headingLevel != null }}
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
          onClick={(e) => {
            e.stopPropagation();
            cycleBlockMarker(props.id);
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
                <span class="prop-key">{k}</span>
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

// Cycle the task marker on a block (OG order), used by the marker chip click.
function cycleBlockMarker(id: string) {
  const { raw } = cycleMarkerSmart(doc.byId[id].raw, workflow(), {
    format: formatForBlockId(id),
    enabled: timetrackingEnabled(),
    withSeconds: logbookWithSecondSupport(),
  });
  setRaw(id, raw, { timetracking: false });
}

// Toggle the task checkbox (OG check/uncheck): open → DONE (rolling a repeater
// forward instead), DONE → the workflow's open marker. Used by the block checkbox.
function toggleBlockCheckbox(id: string) {
  const raw = toggleTaskDone(doc.byId[id].raw, workflow(), {
    format: formatForBlockId(id),
    enabled: timetrackingEnabled(),
    withSeconds: logbookWithSecondSupport(),
  });
  if (raw !== null) setRaw(id, raw, { timetracking: false });
}

function formatForBlockId(id: string): "md" | "org" {
  return pageByName(doc.byId[id]?.page)?.format ?? "md";
}

function ClockBadge(props: { info: LogbookInfo }): JSX.Element {
  const rows = () => props.info.rows.slice().reverse().slice(0, 10);
  return (
    <span class="clock-badge" tabIndex={0}>
      <span class="clock-badge-label">{props.info.summary}</span>
      <span class="clock-tooltip" role="tooltip">
        <table>
          <thead>
            <tr>
              <th>Type</th>
              <th>Start</th>
              <th>End</th>
              <th>Span</th>
            </tr>
          </thead>
          <tbody>
            <For each={rows()}>
              {(r) => (
                <tr>
                  <td>{r.type}</td>
                  <td>{r.start}</td>
                  <td>{r.end ?? ""}</td>
                  <td>{r.span ?? ""}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </span>
    </span>
  );
}

interface AcItem {
  label: string;
  /** Secondary, dimmer line (e.g. the page a block-ref candidate lives on). */
  sub?: string;
  insert?: string;
  caret?: number;
  action?: import("../editor/autocomplete").CommandAction;
  templateNodes?: import("../types").BlockDto[];
  /** A `((block reference))` candidate: insert `((uuid))` and persist id::. */
  blockRef?: { uuid: string; page: string; kind: import("../types").PageKind };
}

/** First visible (non-`key:: value`) line of a block's raw markdown — what the
 *  block-reference picker shows as the candidate's label. */
function blockFirstLine(raw: string): string {
  for (const line of raw.split("\n")) {
    if (!/^\s*[\w-]+:: /.test(line) && line.trim() !== "") return line.trim();
  }
  return "";
}

/** If the caret sits on an in-block markdown list line (`+`/`*`/ordered — NOT the
 *  outline bullet `-`), return its parts, for caret-context list editing. */
// In-block list markers differ by format (see body.tsx): Markdown uses `+`/`*`
// (a leading `-` is the outline bullet), Org uses `-`/`+` (a leading `*` is a
// headline). Numbered works in both.
const LIST_LINE_MD = /^(\s*)([+*]|\d+[.)])(\s+)(\[[ xX]\]\s+)?/;
const LIST_LINE_ORG = /^(\s*)([-+]|\d+[.)])(\s+)(\[[ xX]\]\s+)?/;
function listLineAt(
  text: string,
  caret: number,
  format: "md" | "org" = "md",
): { indent: string; marker: string; hasCheckbox: boolean; lineStart: number; prefixLen: number } | null {
  const lineStart = text.lastIndexOf("\n", caret - 1) + 1;
  let lineEnd = text.indexOf("\n", caret);
  if (lineEnd === -1) lineEnd = text.length;
  const re = format === "org" ? LIST_LINE_ORG : LIST_LINE_MD;
  const m = re.exec(text.slice(lineStart, lineEnd));
  if (!m) return null;
  return { indent: m[1], marker: m[2], hasCheckbox: !!m[4], lineStart, prefixLen: m[0].length };
}

// Small calendar glyph for date chips (SVG, not emoji — emoji tofu on WebKitGTK).
function CalGlyph(): JSX.Element {
  return (
    <svg class="chip-cal" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="4" y="5" width="16" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="2" />
      <line x1="4" y1="9.5" x2="20" y2="9.5" stroke="currentColor" stroke-width="2" />
    </svg>
  );
}

function timeStamp(d = new Date()): string {
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}
// Today's journal page name in the default "MMM do, yyyy" title format the app
// uses (matches logseq-core's JournalDate::title), so [[Today]] resolves.
function todayJournalName(d = new Date()): string {
  const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  const n = d.getDate();
  const a = n % 10;
  const b = n % 100;
  const suffix =
    (a === 1 && b === 11) || (a === 2 && b === 12) || (a === 3 && b === 13)
      ? "th"
      : a === 1
        ? "st"
        : a === 2
          ? "nd"
          : a === 3
            ? "rd"
            : "th";
  return `${months[d.getMonth()]} ${n}${suffix}, ${d.getFullYear()}`;
}

// Template support: session-cached list of templates, dynamic-var substitution,
// and DTO→outline conversion for insertion.
let templateCache: import("../types").TemplateDto[] | null = null;
let templateCacheRev = -1;
async function getTemplates(): Promise<import("../types").TemplateDto[]> {
  // Re-fetch when the graph has changed since the last fetch (keyed on dataRev),
  // so a template just created (here or externally) shows up without a reload.
  const rev = dataRev();
  if (templateCache && templateCacheRev === rev) return templateCache;
  try {
    templateCache = await backend().listTemplates();
    templateCacheRev = rev;
  } catch {
    templateCache = [];
  }
  return templateCache;
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
// Markdown for a freshly saved asset: images embed inline, everything else
// (PDFs included) becomes a link — a .pdf link renders as a clickable chip that
// opens the PDF pane.
// `onSubmit`/`onCancel` (set only by the quick-capture window) repurpose a plain
// Enter / Escape when the autocomplete popup is closed: Enter commits the capture
// instead of splitting the block, Escape dismisses instead of entering
// block-selection. Everything else — autocomplete, slash commands, formatting —
// is the identical page-editing experience because it's the identical component.
export function Editor(props: { id: string }): JSX.Element {
  // Non-null only inside the quick-capture window (see CaptureCtx).
  const cap = useContext(CaptureCtx);
  const sheetCell = useContext(SheetCellContext);
  // Which surface (main pane / a specific sidebar item) this editor lives in —
  // drives edit-focus arbitration when the same block renders in several surfaces.
  const surfaceKey = useContext(SurfaceContext);
  let ref!: HTMLTextAreaElement;
  // Caret/selection stashed when the *window* (not this block) loses focus, so
  // returning to Tine resumes editing exactly where you left off.
  let savedSel: { start: number; end: number } | null = null;
  const node = () => doc.byId[props.id];
  const sheetInitialRaw = sheetCell ? node()?.raw ?? "" : null;
  // Page format drives in-block list markers (`-` is an org bullet, not md).
  const pageFmt = (): "md" | "org" => (pageByName(node().page)?.format === "org" ? "org" : "md");

  // What the textarea shows. Annotation (PDF highlight) blocks expose only their
  // highlight text (all metadata hidden); every other block hides just the
  // built-in id::/collapsed:: lines (like OG). Hidden lines are preserved and
  // reattached on commit.
  const isAnnot = () => isAnnotationBlock(node().raw);
  // Annotation blocks hide ALL properties (edit only the highlight text); every
  // other block hides just the built-in id::/collapsed::. One fence-aware splitter.
  const hideFn = () => (isAnnot() ? hideAll : sheetCell ? isSheetCellHidden : isBuiltinHidden);
  const editorValue = createMemo(() => splitProps(node().raw, hideFn()).visible);
  const editorHeadingLevel = createMemo(() => {
    const visible = editorValue();
    if (visible.includes("\n")) return null;
    return facetsOf(visible, pageFmt()).headingLevel;
  });
  // Live calc preview: when this block is a ```calc fence, show the SAME results
  // panel as the rendered view, recomputed on every keystroke (onInput commits
  // to node().raw live, so editorValue() is current). Matches OG's calculator,
  // which stays live while you type instead of only computing after you exit.
  // A ```calc block edits like OG: the textarea shows ONLY the fence-stripped
  // expressions (calcLive), with a line-number gutter + live results beside it,
  // and the fence is re-added on commit. `calcLive()` is non-null iff this is a
  // calc block (so isCalc()), and `calcRows()` evaluates each expression line.
  const calcLive = createMemo(() => calcSource(editorValue()));
  const isCalc = () => calcLive() !== null;
  const calcRows = createMemo(() => (isCalc() ? evalCalc(calcLive() ?? "") : []));
  const commit = (text: string, opts?: { timetracking?: boolean }) => {
    // For calc, `text` is the bare expressions the user sees — re-fence it.
    // Keep any trailing space the user left (or that a `/priority` insert added as
    // a typing convenience) in the live buffer — OG keeps it while you edit and
    // only trims when the block is written to disk. We do the SAME: the trailing
    // trim now lives at the save boundary (toDto in store.ts), not here. Trimming
    // here re-synced the reactive textarea (`value={editorValue()}`) to the
    // trimmed text on every keystroke, so backspacing to a trailing space ate the
    // space out from under the caret — the block-eats-the-space bug.
    const visible = isCalc() ? wrapCalc(text) : text;
    const next = joinProps(visible, splitProps(node().raw, hideFn()).hidden);
    // No-op commit (focus/blur with no real edit, or text that reconstructs the
    // identical raw): don't mark the page dirty or push undo — avoids churn and
    // can't rewrite the block's bytes.
    if (next === node().raw) return;
    setRaw(props.id, next, opts);
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
  };

  const updateAutocomplete = async () => {
    const t = detectTrigger(ref.value, ref.selectionStart);
    if (!t) {
      closeAc();
      return;
    }
    setAc(t);
    setAcIndex(0);
    if (t.kind === "command") {
      const q = t.query;
      const tmpls = await getTemplates();
      const cur = ac();
      if (!cur || cur.start !== t.start) return; // trigger changed while awaiting
      // Commands AND templates in one fuzzy-ranked list, so a strong template
      // match can outrank a weak command (and vice-versa). Empty query (bare `/`)
      // lists all commands in defined order, no templates. `idx` preserves the
      // defined order — commands before templates — as the stable tiebreaker.
      const showAllTemplates = !!q && "template".startsWith(q.toLowerCase()); // /t…/template lists them all
      const scored: { item: AcItem; s: number; idx: number }[] = [];
      COMMANDS.forEach((c, i) => {
        const s = q ? commandScore(q, c) : 1;
        if (s > 0)
          scored.push({ item: { label: c.label, insert: c.insert, caret: c.caret, action: c.action }, s, idx: i });
      });
      if (q) {
        tmpls.forEach((tp, j) => {
          const s = showAllTemplates ? 1 : fuzzyScore(q, tp.name);
          if (s > 0)
            scored.push({ item: { label: `Template: ${tp.name}`, templateNodes: tp.blocks }, s, idx: COMMANDS.length + j });
        });
      }
      scored.sort((a, b) => b.s - a.s || a.idx - b.idx);
      setAcItems(scored.map((x) => x.item));
      return;
    }
    if (t.kind === "block") {
      // `((` → full-text search for a block to reference, grouped by page. An
      // empty query (bare `((`) returns nothing — the popup stays hidden until
      // the user types. Selecting inserts `((uuid))` (see selectAc).
      const groups = await backend().search(t.query, 8);
      const cur = ac();
      if (!cur || cur.start !== t.start) return; // trigger changed while awaiting
      const items: AcItem[] = [];
      for (const g of groups) {
        for (const b of g.blocks) {
          items.push({
            label: blockFirstLine(b.raw) || g.page,
            sub: g.page,
            blockRef: { uuid: b.id, page: g.page, kind: g.kind },
          });
        }
      }
      setAcItems(items);
      return;
    }
    const pages = await backend().quickSwitch(t.query, 8);
    const cur = ac();
    if (!cur || cur.start !== t.start) return; // trigger changed while awaiting
    // Default (first / Enter) item when the query is neither blank nor an exact
    // existing page. OG behavior (linkFirstMatch OFF): "Create <typed>" leads, so
    // a fresh #tag or [[page]] + Enter MAKES it — even when it prefix-/fuzzy-
    // matches an existing page (e.g. #book → a "Books" page) — and the matches
    // follow (arrow down to link instead). With linkFirstMatch ON: the first
    // match leads (Enter LINKS) and "Create" goes to the end. Either way, no
    // create option for a blank query or an exact match.
    const q = t.query.trim();
    const pageItem = (name: string): AcItem =>
      t.kind === "page"
        ? { label: name, insert: pageInsert(name) }
        : { label: `#${name}`, insert: tagInsert(name) }; // tag context reads "#name"
    const createItem: AcItem =
      t.kind === "page"
        ? { label: `Create "${q}"`, insert: pageInsert(q) }
        : { label: `Create #${q}`, insert: tagInsert(q) };
    const matches = pages.map((p) => pageItem(p.name));
    const exact = pages.some((p) => p.name.toLowerCase() === q.toLowerCase());
    setAcItems(
      orderAcItems(matches, createItem, { hasQuery: !!q, exact, linkFirst: linkFirstMatch() })
    );
  };

  // Apply a pure text edit (format toggle / kill motion) to the textarea and
  // restore the resulting selection.
  const applyEdit = (ed: Edit) => {
    commit(ed.text);
    queueMicrotask(() => {
      ref.value = ed.text;
      ref.setSelectionRange(ed.start, ed.end);
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
  const updateSel = () => setHasSel(ref.selectionStart !== ref.selectionEnd);
  const fmt = (left: string, right?: string) => {
    applyEdit(toggleWrap(ref.value, ref.selectionStart, ref.selectionEnd, left, right));
    queueMicrotask(updateSel);
  };
  const doLink = () => {
    applyEdit(insertLink(ref.value, ref.selectionStart, ref.selectionEnd));
    setHasSel(false);
  };

  // Insert `text` in place of the active trigger and restore the caret. If the
  // completion ends with a closing pair (`]]`/`))`/`}}`) and the same pair
  // already sits right after the caret (e.g. from a `[[ ]]` autopair or editing
  // inside an existing ref), swallow it so we don't end up with `[[name]]]]`.
  const replaceTrigger = (text: string, caret?: number) => {
    const t = ac();
    if (!t) return;
    let end = t.end;
    for (const pair of ["]]", "))", "}}"]) {
      if (text.endsWith(pair) && ref.value.slice(t.end, t.end + 2) === pair) {
        end = t.end + 2;
        break;
      }
    }
    const r = applyCompletion(ref.value, t.start, end, text, caret);
    commit(r.raw);
    closeAc();
    queueMicrotask(() => {
      ref.value = r.raw;
      ref.setSelectionRange(r.caret, r.caret);
      ref.focus();
      autosize();
    });
  };

  // Open the native file picker, copy the chosen file into assets/, and insert
  // its markdown at the caret. Uses the Tauri dialog plugin + import_asset.
  // Seed + insert an asset from raw bytes at the caret, then persist to assets/
  // in the background (repointing the link if the backend de-dups the name).
  // Shared by clipboard-image paste and mobile capture (camera / voice memo).
  const insertAssetBytes = (bytes: Uint8Array, origName?: string) => {
    const candidate = assetFileName(origName);
    // Cache key is the bare filename — assetRelPath() strips the `assets/` prefix
    // before loadAssetBlob() (see render/inline.tsx). Seed it so the asset renders
    // instantly, before the disk write lands.
    seedAssetBlob(candidate, bytes);
    const md = assetMarkdown(candidate);
    const start = ref.selectionStart;
    const newRaw = ref.value.slice(0, start) + md + ref.value.slice(ref.selectionEnd);
    const fixupTarget = insertedAssetMarkdownTarget(newRaw, md, start);
    commit(newRaw);
    const pos = start + md.length;
    queueMicrotask(() => {
      ref.value = newRaw;
      ref.setSelectionRange(pos, pos);
      ref.focus();
      autosize();
    });
    void (async () => {
      let stored: string;
      try {
        stored = await backend().saveAsset(candidate, bytes);
      } catch {
        pushToast(`Couldn’t save to assets/`, "error");
        return;
      }
      // The backend de-dups a colliding name (e.g. two inserts in the same second)
      // to `<name>_1.ext`; repoint the link + blob to the ACTUAL stored file so the
      // block never references a wrong/missing asset and the real file isn't orphaned.
      if (stored && stored !== candidate) {
        seedAssetBlob(stored, bytes);
        const cur = node().raw;
        const fixed = replaceInsertedAssetMarkdown(cur, candidate, stored, fixupTarget);
        if (fixed !== cur) commit(fixed);
      }
    })();
  };

  // Mobile: take/pick a photo (Android camera plugin) → insert at the caret.
  const capturePhotoCmd = async () => {
    let res;
    try {
      res = await backend().capturePhoto();
    } catch (err) {
      pushToast(`Couldn’t capture a photo (${String(err)})`, "error");
      return;
    }
    if (res.status === "ok" && res.data) insertAssetBytes(base64ToBytes(res.data), `photo.${res.ext || "jpg"}`);
  };

  // Mobile: toggle voice-memo recording. First tap starts (prompts for mic
  // permission); second tap stops and inserts the recorded audio at the caret.
  const voiceMemoToggle = async () => {
    if (isRecordingAudio()) {
      setRecordingAudio(false);
      let res;
      try {
        res = await backend().stopRecording();
      } catch (err) {
        pushToast(`Couldn’t save the recording (${String(err)})`, "error");
        return;
      }
      if (res.status === "ok" && res.data) insertAssetBytes(base64ToBytes(res.data), `voice-memo.${res.ext || "m4a"}`);
      return;
    }
    let res;
    try {
      res = await backend().startRecording();
    } catch (err) {
      pushToast(`Couldn’t start recording (${String(err)})`, "error");
      return;
    }
    if (res.status === "recording") {
      setRecordingAudio(true);
      pushToast("Recording… tap the mic again to stop", "info");
    }
  };

  const uploadAsset = async () => {
    const path = await backend().pickFile();
    if (!path) return;
    try {
      // Store with a timestamped name (keeps the original + a sortable insert time).
      const orig = path.split(/[\\/]/).pop() || undefined;
      const saved = await backend().importAsset(path, assetFileName(orig));
      const md = assetMarkdown(saved);
      const pos = ref.selectionStart;
      const nr = ref.value.slice(0, pos) + md + ref.value.slice(pos);
      commit(nr); // reattach hidden id::/collapsed:: (nr is visible-only text)
      const c = pos + md.length;
      queueMicrotask(() => {
        ref.value = nr;
        ref.setSelectionRange(c, c);
        ref.focus();
        autosize();
      });
    } catch {
      // ignore failed imports
    }
  };

  const selectAc = (item: AcItem) => {
    const t = ac();
    if (!t) return;
    if (item.blockRef) {
      // Insert `((uuid))` now (resolves in-session via the in-memory uuid), then
      // durably stamp the target's id:: in the background so it survives restart.
      const { uuid, page, kind } = item.blockRef;
      replaceTrigger(`((${uuid}))`);
      void persistBlockRefTarget(uuid, page, kind);
      return;
    }
    if (item.templateNodes) {
      // Drop the "/name" trigger text, then insert the template's blocks (with
      // dynamic vars resolved). If the host block is now empty, replace it.
      const r = applyCompletion(ref.value, t.start, t.end, "");
      commit(r.raw);
      closeAc();
      const nodes = item.templateNodes.map((n) => templateToOutline(n, doc.byId[props.id]?.page));
      const wasEmpty =
        doc.byId[props.id].raw.trim() === "" && doc.byId[props.id].children.length === 0;
      const lastId = insertOutlineAfter(props.id, nodes);
      if (wasEmpty) deleteBlock(props.id);
      startEditing(lastId, doc.byId[lastId].raw.length);
      return;
    }
    switch (item.action) {
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
      case "query-builder": {
        // Insert an empty query, commit it, and drop straight to the rendered
        // view so the visual builder appears — then flag this block so the
        // builder opens its add-filter picker on mount.
        const r = applyCompletion(ref.value, t.start, t.end, "{{query }}");
        commit(r.raw);
        closeAc();
        setQueryBuilderAutoOpen(props.id);
        endEdit("query-builder");
        return;
      }
      case "page-props": {
        // Drop the trigger text, then open the page-properties panel for the
        // page this block lives on (anchored under the editor).
        replaceTrigger("");
        const rect = ref.getBoundingClientRect();
        openPageProps(doc.byId[props.id].page, rect.left, rect.bottom + 4);
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
        replaceTrigger(pageInsert(todayJournalName()));
        return;
      case "upload-asset":
        replaceTrigger(""); // drop the "/upload" trigger text
        uploadAsset();
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
    ref.style.height = "auto";
    ref.style.height = `${ref.scrollHeight}px`;
  };
  let autosizeRaf: number | undefined;
  const autosize = () => {
    if (autosizeRaf !== undefined) return; // already scheduled this frame
    autosizeRaf = requestAnimationFrame(() => {
      autosizeRaf = undefined;
      resizeNow();
    });
  };

  const focusNow = () => {
    const want = takeCaretFor(props.id);
    ref.focus();
    const v = ref.value;
    let offset: number;
    if (want == null) {
      offset = editorValue().length;
    } else if (typeof want === "number") {
      offset = want;
    } else {
      // Cross-block Up/Down: land `col` chars into this (target) block's FIRST
      // (Down) or LAST (Up) source line, clamped to that line — OG parity. Resolved
      // here against the target's real value, so multi-line planning blocks work.
      if (want.edge === "first") {
        const nl = v.indexOf("\n");
        const lineLen = nl === -1 ? v.length : nl;
        offset = Math.min(want.col, lineLen);
      } else {
        const lineStart = v.lastIndexOf("\n") + 1;
        offset = lineStart + Math.min(want.col, v.length - lineStart);
      }
    }
    const o = Math.min(offset, v.length);
    ref.setSelectionRange(o, o);
  };
  onMount(() => {
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
  });

  let acTimer: ReturnType<typeof setTimeout> | undefined;
  const onInput = (e: InputEvent) => {
    // Editor keystroke post-processing on a single inserted char (not paste/IME/
    // delete). All branches edit ref.value BEFORE commit so the store sees it.
    if (e.inputType === "insertText" && e.data && e.data.length === 1 && !e.isComposing) {
      const ch = e.data;
      let handled = false;
      if (autoPairing()) {
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
    }
    commit(ref.value);
    autosize();
    // Close the popup synchronously when the trigger ends (instant), but debounce
    // the page/template IPC fetch so holding down a key doesn't fire a backend
    // round-trip per character.
    if (!detectTrigger(ref.value, ref.selectionStart)) {
      clearTimeout(acTimer);
      closeAc();
      return;
    }
    clearTimeout(acTimer);
    acTimer = setTimeout(() => void updateAutocomplete(), 90);
  };

  // Move the block up/down among siblings, keeping edit mode + caret (the DOM
  // reorder briefly blurs the textarea; cross-day it remounts).
  const moveBlockCmd = (e: KeyboardEvent, dir: 1 | -1): boolean => {
    e.preventDefault();
    const start = ref.selectionStart;
    commit(ref.value);
    setBlockMoving(true);
    startEditing(props.id, start);
    void moveBlockFeed(props.id, dir).then(() => {
      requestAnimationFrame(() => {
        if (ref.isConnected) {
          ref.focus();
          const o = Math.min(start, ref.value.length);
          ref.setSelectionRange(o, o);
        }
        setBlockMoving(false);
      });
    });
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
    selectBlock(props.id);
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

  // Editor command handlers keyed by command id (see keybindings.ts). Each does
  // its own preventDefault when it handles the event and returns whether it did
  // (false → fall through to native handling). Read ref fresh at call time.
  const runEditorCmd: Record<string, (e: KeyboardEvent) => boolean> = {
    "editor/bold": (e) => { e.preventDefault(); applyEdit(toggleWrap(ref.value, ref.selectionStart, ref.selectionEnd, "**")); return true; },
    "editor/italics": (e) => { e.preventDefault(); applyEdit(toggleWrap(ref.value, ref.selectionStart, ref.selectionEnd, "*")); return true; },
    "editor/strike-through": (e) => { e.preventDefault(); applyEdit(toggleWrap(ref.value, ref.selectionStart, ref.selectionEnd, "~~")); return true; },
    "editor/highlight": (e) => { e.preventDefault(); applyEdit(toggleWrap(ref.value, ref.selectionStart, ref.selectionEnd, "==")); return true; },
    "editor/insert-link": (e) => { e.preventDefault(); applyEdit(insertLink(ref.value, ref.selectionStart, ref.selectionEnd)); return true; },
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
    "editor/cycle-todo": (e) => { e.preventDefault(); cycleTodoCmd(); return true; },
    "editor/indent": (e) => {
      e.preventDefault();
      // On an in-block list line, Tab nests the LIST ITEM (intra-block), not the block.
      const ll = listLineAt(ref.value, ref.selectionStart, pageFmt());
      if (ll) { nudgeListItem(ll, +2); return true; }
      commit(ref.value); indentBlock(props.id, ref.selectionStart); return true;
    },
    "editor/outdent": (e) => {
      e.preventDefault();
      const ll = listLineAt(ref.value, ref.selectionStart, pageFmt());
      if (ll && ll.indent.length > 0) { nudgeListItem(ll, -2); return true; }
      commit(ref.value); outdentBlock(props.id, ref.selectionStart); return true;
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
    return (doc.byId[id]?.children ?? []).find((child) => blockIsGridView(child)) ?? null;
  };
  const sheetVisibleLength = (id: string): number =>
    splitProps(doc.byId[id]?.raw ?? "", isSheetCellHidden).visible.length;
  const deepestLastSheetOutline = (id: string): string => {
    let cur = id;
    for (;;) {
      if (sheetFaceGridId(cur)) return cur;
      const children = doc.byId[cur]?.children ?? [];
      if (!children.length) return cur;
      cur = children[children.length - 1];
    }
  };
  const nextSheetOutline = (id: string, hostId: string): string | null => {
    if (!sheetFaceGridId(id)) {
      const firstChild = doc.byId[id]?.children[0];
      if (firstChild) return firstChild;
    }
    let cur = id;
    while (cur !== hostId) {
      const parent = doc.byId[cur]?.parent ?? null;
      if (!parent) return null;
      const siblings = doc.byId[parent]?.children ?? [];
      const idx = siblings.indexOf(cur);
      if (idx >= 0 && idx + 1 < siblings.length) return siblings[idx + 1];
      cur = parent;
    }
    return null;
  };
  const prevSheetOutline = (id: string, hostId: string): string | "host" | null => {
    const parent = doc.byId[id]?.parent ?? null;
    if (!parent) return null;
    const siblings = doc.byId[parent]?.children ?? [];
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
      if (nestedGridId) selectTopRowSeamAfterEdit(nestedGridId);
      else {
        const hostId = cellBlockId(sheetCell);
        const next = hostId
          ? props.id === hostId
            ? doc.byId[hostId]?.children[0] ?? null
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
    if (!e.ctrlKey && !e.metaKey && !e.altKey && (e.key === "Tab" || e.code === "Tab")) {
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
    const start = ref.selectionStart;
    const end = ref.selectionEnd;
    const raw = ref.value;

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
      if (e.key === "Enter" || e.code === "Tab") {
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

    // Auto-pair wrap on a SELECTION (OG parity, always-on — independent of the
    // opt-in empty-caret auto-pairing). Typing any of `SELECTION_WRAP` around
    // selected text wraps it, keeping the selection: `*`/`~`/`=` etc. so a second
    // press gives `**bold**`/`~~strike~~`/`==highlight==`, and `[`/`(` so `[[sel]]`
    // makes a page ref and `((sel))` a block ref — the doubling bracket then opens
    // the matching search seeded with the selection, so Enter links it to an
    // existing page/block or creates it. Modifier-free single chars only, so it
    // never shadows Ctrl/Cmd editor commands or IME composition.
    if (
      start !== end &&
      !e.ctrlKey && !e.metaKey && !e.altKey && !e.isComposing &&
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
      void ensureBlockId(props.id).then((uuid) => {
        if (uuid) {
          void backend().writeText(`((${uuid}))`);
          pushToast("Copied block ref", "success");
        } else {
          pushToast("Couldn't save the block id — reference not copied.", "error");
        }
      });
      return;
    }

    // Configurable editor commands → one dispatch through the handler table
    // (runEditorCmd) instead of ~20 sequential matchesCommand checks. A handler
    // returns false to fall through — select-block does this off the block edge
    // so the textarea extends the selection by a wrapped line.
    const cmd = editorCommandFor(e);
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

    if (e.key === "Enter" && e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      softNewlineCmd();
      return;
    }

    if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
      // In a calc block, Enter adds a new expression line (stays in the block) —
      // let the textarea insert the newline natively, like OG.
      if (isCalc()) return;
      e.preventDefault();
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
        startEditing(newId, 0);
      } else {
        splitBlock(props.id, start);
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
        // Never merge a highlight or calc block away (their structure must stay).
        if (isAnnot() || isCalc()) return;
        commit(raw);
        if (mergeWithPrev(props.id)) {
          e.preventDefault();
          return;
        }
        const n = doc.byId[props.id];
        const next = nextVisible(props.id);
        if (n && splitProps(n.raw, hideFn()).visible.trim() === "" && n.children.length === 0 && next) {
          e.preventDefault();
          deleteBlock(props.id);
          startEditing(next, 0);
        }
      }
    } else if (e.key === "ArrowUp" && !e.shiftKey) {
      // Leave for the previous block only from the FIRST visual row; otherwise
      // let the textarea move the caret up one wrapped line. (A long single line
      // has no `\n` but still wraps — the source-`\n` test alone wrongly jumped
      // to the parent from the second visual row.)
      const before = raw.slice(0, start);
      if (!before.includes("\n") && caretAtFirstRow(ref, start)) {
        const prev = prevVisible(props.id);
        if (prev) {
          e.preventDefault();
          // OG parity: keep the caret's column — land it that many chars into the
          // LAST source line of the previous block (clamped). No sticky goal column.
          startEditing(prev, { col: start - (before.lastIndexOf("\n") + 1), edge: "last" });
        }
      }
    } else if (e.key === "ArrowDown" && !e.shiftKey) {
      const after = raw.slice(start);
      if (!after.includes("\n") && caretAtLastRow(ref, start)) {
        // OG parity: keep the caret's column — land it that many chars into the
        // FIRST source line of the next block (clamped). Column is measured within
        // the CURRENT (last) source line, so multi-line planning blocks work too.
        const col = start - (raw.slice(0, start).lastIndexOf("\n") + 1);
        const next = nextVisible(props.id);
        if (next) {
          e.preventDefault();
          startEditing(next, { col, edge: "first" });
        } else {
          // No next LOADED block. In the journal feed, pull in the next day so
          // Down-arrow keeps going past the loaded window (previously only a
          // mouse-wheel scroll grew the feed). Non-feed pages resolve to null → a
          // harmless no-op. Async: flush first, then step into the new day.
          e.preventDefault();
          commit(raw);
          void nextVisibleOrExtend(props.id).then((n) => n && startEditing(n, { col, edge: "first" }));
        }
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      selectBlock(props.id); // exit editing into block-selection mode
    }
  };

  const onBlur = () => {
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
    commit(normalizePlanning(ref.value, pageFmt()));
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

  // Paste an image from the clipboard. WebKitGTK's <textarea> paste event does
  // not expose image data, so we read the OS clipboard directly (Tauri plugin)
  // and insert an asset link. Text paste proceeds normally (no preventDefault);
  // for an image-only clipboard there is no text to paste.
  const onPaste = (e: ClipboardEvent) => {
    const text = e.clipboardData?.getData("text/plain") ?? "";
    // Multiline text pastes as a block outline (Logseq behavior).
    if (text.includes("\n")) {
      e.preventDefault();
      const start = ref.selectionStart;
      const end = ref.selectionEnd;
      if (sheetCell || isCalc() || caretInFence(ref.value, start)) {
        const newRaw = ref.value.slice(0, start) + text + ref.value.slice(end);
        commit(newRaw);
        const pos = start + text.length;
        queueMicrotask(() => {
          ref.value = newRaw;
          ref.setSelectionRange(pos, pos);
          autosize();
        });
        return;
      }
      const nodes = parseOutline(text);
      if (!nodes.length) return;
      commit(ref.value);
      const wasEmpty =
        doc.byId[props.id].raw.trim() === "" && doc.byId[props.id].children.length === 0;
      const lastId = insertOutlineAfter(props.id, nodes);
      if (wasEmpty) deleteBlock(props.id);
      startEditing(lastId, doc.byId[lastId].raw.length);
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
        !caretInFence(ref.value, start)
      ) {
        e.preventDefault();
        applyEdit(wrapLink(ref.value, start, end, url, pageFmt()));
        queueMicrotask(updateSel);
        return;
      }
    }
    // Single-line/no text: maybe an image on the OS clipboard. Show an immediate
    // "Pasting image…" hint when the clipboard clearly holds one (so there's no
    // dead 1–2s), then render it instantly from the in-memory bytes and write to
    // disk in the background.
    const looksImage =
      Array.from(e.clipboardData?.items ?? []).some((it) => it.type.startsWith("image/")) ||
      (e.clipboardData?.types ?? []).some((t) => t.startsWith("image/") || t === "Files");
    const toastId = looksImage ? pushToast("Pasting image…", "info") : 0;
    void (async () => {
      let bytes: Uint8Array | null = null;
      try {
        bytes = await backend().readClipboardImage();
      } finally {
        if (toastId) dismissToast(toastId);
      }
      if (!bytes) return;
      insertAssetBytes(bytes);
    })();
  };

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
        classList={{ [`h${editorHeadingLevel()}`]: editorHeadingLevel() != null }}
        spellcheck={spellcheckEnabled()}
        value={isCalc() ? (calcLive() ?? "") : editorValue()}
        placeholder={cap?.bulletHint?.()}
        onInput={onInput}
        onKeyDown={onKeyDown}
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
        <div class="calc-results" aria-hidden="true">
          <For each={calcRows()}>
            {(r) => (
              <div class="calc-out" classList={{ "calc-error": !!r.error }}>
                {r.output ?? ""}
              </div>
            )}
          </For>
        </div>
      </Show>
      <Show when={hasSel()}>
        <div class="sel-toolbar" onMouseDown={(e) => e.preventDefault()}>
          <button title="Bold (mod+b)" onClick={() => fmt("**")}><b>B</b></button>
          <button title="Italic (mod+i)" onClick={() => fmt("*")}><i>I</i></button>
          <button title="Strikethrough" onClick={() => fmt("~~")}><s>S</s></button>
          <button title="Highlight" onClick={() => fmt("==")}><mark>H</mark></button>
          <button title="Link" onClick={doLink}>
            <svg viewBox="0 0 24 24" width="13" height="13" aria-hidden="true">
              <path d="M9 15l6-6M10 6l1-1a4 4 0 015.7 5.7l-1 1M14 18l-1 1a4 4 0 01-5.7-5.7l1-1"
                fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
            </svg>
          </button>
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

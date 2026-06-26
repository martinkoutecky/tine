import { Show, Switch, Match, For, createMemo, createSignal, createContext, useContext, createUniqueId, createEffect, onMount, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import {
  detectTrigger,
  applyCompletion,
  pageInsert,
  tagInsert,
  COMMANDS,
  commandScore,
  fuzzyScore,
  type Trigger,
} from "../editor/autocomplete";
import {
  doc,
  pageByName,
  editingId,
  editingOwner,
  startEditing,
  setRaw,
  setEditingId,
  splitBlock,
  indentBlock,
  outdentBlock,
  mergeWithPrev,
  toggleCollapse,
  setCollapsed,
  takeCaretFor,
  prevVisible,
  nextVisible,
  insertOutlineAfter,
  deleteBlock,
  moveBlock,
  moveBlockFeed,
  selectBlock,
  moveSelection,
  isSelected,
  persistentBlockRef,
  persistBlockRefTarget,
  isBlockMoving,
  setBlockMoving,
} from "../store";
import { parseOutline } from "../editor/outline";
import {
  toggleWrap,
  insertLink,
  killLineBefore,
  killLineAfter,
  wordForward,
  wordBackward,
  killWordForward,
  killWordBackward,
  setPriority,
  type Edit,
} from "../editor/format";
import { blockView } from "../render/block";
import { BodyContent } from "../render/body";
import { assetMarkdown, assetFileName } from "../media";
import { calcSource, wrapCalc, evalCalc } from "../editor/calc";
import { QueryMacro, EmbedMacro } from "./Macro";
import { workflow, zoomInto, openContextMenu, openDatePicker, openBlockInSidebar, graphMeta, dataRev, setQueryBuilderAutoOpen, openPageProps } from "../ui";
import { openPageInNewTab } from "../router";
import { editorCommandFor } from "../keybindings";
import { cycleMarkerSmart } from "../editor/repeat";
import { applyTemplateVars } from "../editor/templateVars";
import { caretAtFirstRow, caretAtLastRow } from "../editor/caretRows";
import { splitProps, joinProps, isBuiltinHidden, hideAll } from "../editor/properties";
import { isAnnotationBlock, annotationInfo } from "../editor/annotation";
import { AnnotationBody } from "./AnnotationBody";

// Detect a block whose entire body is a single {{query}} / {{embed}} macro.
function detectMacro(lines: string[]): { kind: "query" | "embed"; inner: string } | null {
  const text = lines.join("\n").trim();
  const m = /^\{\{(query|embed)\b([\s\S]*)\}\}$/.exec(text);
  if (!m) return null;
  return { kind: m[1] as "query" | "embed", inner: `${m[1]}${m[2]}` };
}

// Internal/metadata properties hidden from the rendered properties area.
const INTERNAL_PROPS = new Set([
  "id", "collapsed", "hl-page", "hl-color", "hl-type", "ls-type",
  "background-color", "logseq.order-list-type",
  // OG hidden-built-in-properties (don't render these in the properties area).
  "heading", "title", "filters", "created-at", "updated-at", "last-modified-at",
  "query-table", "query-properties", "query-sort-by", "query-sort-desc", "logseq.tldraw.shape",
]);

// Logseq's built-in block background colors → a soft tint for rendering.
const BLOCK_BG: Record<string, string> = {
  yellow: "rgba(251,230,158,0.45)", red: "rgba(245,163,163,0.4)", pink: "rgba(243,176,212,0.4)",
  green: "rgba(166,227,180,0.4)", blue: "rgba(168,201,240,0.4)", purple: "rgba(205,180,238,0.4)",
  gray: "rgba(211,214,218,0.5)",
};

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
      setEditingId(null);
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
}
export const CaptureCtx = createContext<CaptureApi | null>(null);

export function Block(props: { id: string }): JSX.Element {
  const node = () => doc.byId[props.id];
  // Unique per rendered instance, so when one block uuid appears in several
  // surfaces only the instance that was clicked mounts the editor (the rest stay
  // rendered and reflect edits live). null owner = unscoped (keyboard nav).
  const instanceId = createUniqueId();
  const editing = () =>
    editingId() === props.id && (editingOwner() === null || editingOwner() === instanceId);
  const hasChildren = () => node().children.length > 0;
  const collapsed = () => node().collapsed;
  // An org page Tine can't round-trip is shown but NOT editable (Tine must never
  // rewrite it). Clicking a block doesn't enter the editor on such a page.
  const readOnly = () => pageByName(node().page)?.readOnly ?? false;

  return (
    <div class="ls-block" classList={{ collapsed: collapsed() }} data-block-id={props.id}>
      <div
        class="block-main"
        classList={{
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
            classList={{ "bullet-closed": collapsed() && hasChildren() }}
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
            <span class="bullet" />
          </span>
        </div>

        <div
          class="block-content-wrapper"
          classList={{ "read-only": readOnly() }}
          onClick={() => {
            // Click anywhere in the row (not on a link) starts editing — and
            // claims the editor for THIS instance. Read-only org pages don't edit.
            if (!editing() && !readOnly())
              startEditing(props.id, doc.byId[props.id].raw.length, instanceId);
          }}
        >
          <Show when={editing()} fallback={<Rendered id={props.id} owner={instanceId} />}>
            <Editor id={props.id} />
          </Show>
        </div>
      </div>

      <Show when={hasChildren() && !collapsed()}>
        <div class="block-children-container">
          <div class="block-children-left-border" />
          <div class="block-children" classList={{ ordered: /(?:^|\n)logseq\.order-list-type:: ?number/.test(node().raw) }}>
            <For each={node().children}>{(cid) => <Block id={cid} />}</For>
          </div>
        </div>
      </Show>
    </div>
  );
}

// Walk `root`'s rendered text in document order (treating <br> as "\n") and
// report the running text plus the offset that corresponds to a clicked
// (container, off) from caretRangeFromPoint. The caller only trusts the offset
// when this text equals the editor text 1:1 — so hidden markdown markers, a
// heading `#`, or filtered property/scheduled lines (which make rendered ≠ raw)
// can't misplace the caret; those blocks fall back to end-of-block.
function renderedCaret(root: Node, container: Node, off: number): { text: string; caret: number | null } {
  let text = "";
  let caret: number | null = null;
  const walk = (n: Node) => {
    if (n.nodeType === Node.TEXT_NODE) {
      if (n === container) caret = text.length + off;
      text += n.textContent ?? "";
      return;
    }
    if (n === container && caret === null) caret = text.length; // element container
    if ((n as Element).tagName === "BR") {
      text += "\n";
      return;
    }
    for (const c of Array.from(n.childNodes)) walk(c);
  };
  walk(root);
  return { text, caret };
}

function Rendered(props: { id: string; owner?: string }): JSX.Element {
  const node = () => doc.byId[props.id];
  const view = createMemo(() => blockView(node().raw));
  const fmt = () => pageByName(node().page)?.format ?? "md";
  const readOnly = () => pageByName(node().page)?.readOnly ?? false;

  const macro = createMemo(() => detectMacro(view().lines));

  // PDF highlight (annotation) blocks render a colored, clickable swatch
  // (AnnotationBody) that opens the PDF at the highlight's page; notes go in
  // child blocks. The detection + rendering live in editor/annotation +
  // components/AnnotationBody.
  const annotation = createMemo(() => annotationInfo(view().properties));

  // Click edits the block, placing the caret WHERE you clicked (not at the end).
  // We map the click to the editor text via caretRangeFromPoint, but only trust
  // it when the rendered text matches the editor text exactly — i.e. a plain
  // block. Formatted/heading/property blocks (rendered ≠ raw) fall back to end.
  let contentRef: HTMLDivElement | undefined;
  const clickOffset = (e: MouseEvent): number | null => {
    if (!contentRef) return null;
    const d = document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
    const range = d.caretRangeFromPoint?.(e.clientX, e.clientY);
    if (!range) return null;
    const { text, caret } = renderedCaret(contentRef, range.startContainer, range.startOffset);
    if (caret == null) return null;
    const editorText = splitProps(node().raw, isBuiltinHidden).visible;
    if (text !== editorText) return null; // not a plain block — don't risk a wrong offset
    return Math.min(caret, editorText.length);
  };
  // For annotation blocks the editor shows only the highlight text (metadata
  // stays hidden); the colored prefix still jumps to the PDF.
  const onClick = (e: MouseEvent) => {
    e.stopPropagation();
    if (readOnly()) return; // read-only org page — never enter the editor
    startEditing(props.id, clickOffset(e) ?? node().raw.length, props.owner ?? null);
  };

  const displayProps = () => {
    const extra = graphMeta()?.block_hidden_properties ?? [];
    return view().properties.filter(([k]) => !INTERNAL_PROPS.has(k) && !extra.includes(k));
  };
  const bgColor = () => {
    const v = view().properties.find(([k]) => k === "background-color")?.[1];
    return v ? BLOCK_BG[v] ?? v : undefined;
  };

  const body = (
    <Show when={annotation()} fallback={<BodyContent lines={view().lines} blockId={props.id} format={fmt()} />}>
      <AnnotationBody
        color={annotation()!.color}
        hlPage={annotation()!.hlPage}
        line={view().lines[0]}
        page={node().page}
      />
    </Show>
  );

  return (
    <Show
      when={!macro()}
      fallback={
        <div class="block-content macro-host" onClick={onClick}>
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
      classList={{ done: view().done, "has-bg": !!bgColor(), [`heading h${view().headingLevel ?? ""}`]: view().headingLevel != null }}
      style={bgColor() ? { background: bgColor() } : undefined}
      onClick={onClick}
    >
      <Show when={view().marker}>
        <span
          class={`block-marker marker-${view().marker?.toLowerCase()}`}
          onClick={(e) => {
            e.stopPropagation();
            cycleBlockMarker(props.id);
          }}
        >
          {view().marker}
        </span>{" "}
      </Show>
      <Show when={view().priority}>
        <span class={`block-priority priority-${view().priority}`}>[#{view().priority}]</span>{" "}
      </Show>
      <Show when={view().headingLevel} fallback={body}>
        {(() => {
          const H = `h${view().headingLevel}`;
          return <span class={`heading-text ${H}`}>{body}</span>;
        })()}
      </Show>
      <Show when={view().scheduled}>
        <span
          class="date-chip scheduled"
          title="Scheduled — click to change"
          onClick={(e) => {
            e.stopPropagation();
            openDatePicker(props.id, "scheduled", e.clientX, e.clientY);
          }}
        >
          <CalGlyph /> {view().scheduled}
        </span>
      </Show>
      <Show when={view().deadline}>
        <span
          class="date-chip deadline"
          title="Deadline — click to change"
          onClick={(e) => {
            e.stopPropagation();
            openDatePicker(props.id, "deadline", e.clientX, e.clientY);
          }}
        >
          <CalGlyph /> {view().deadline}
        </span>
      </Show>
      <Show when={displayProps().length > 0}>
        <span class="block-properties">
          <For each={displayProps()}>
            {([k, v]) => (
              <span class="prop">
                <span class="prop-key">{k}</span>
                <span class="prop-value"> {v}</span>
              </span>
            )}
          </For>
        </span>
      </Show>
    </div>
    </Show>
  );
}

// Cycle the task marker on a block (OG order), used by the marker chip click.
function cycleBlockMarker(id: string) {
  const { raw } = cycleMarkerSmart(doc.byId[id].raw, workflow());
  setRaw(id, raw);
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
function listLineAt(
  text: string,
  caret: number,
): { indent: string; marker: string; hasCheckbox: boolean; lineStart: number; prefixLen: number } | null {
  const lineStart = text.lastIndexOf("\n", caret - 1) + 1;
  let lineEnd = text.indexOf("\n", caret);
  if (lineEnd === -1) lineEnd = text.length;
  const m = /^(\s*)([+*]|\d+[.)])(\s+)(\[[ xX]\]\s+)?/.exec(text.slice(lineStart, lineEnd));
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
  let ref!: HTMLTextAreaElement;
  // Caret/selection stashed when the *window* (not this block) loses focus, so
  // returning to Tine resumes editing exactly where you left off.
  let savedSel: { start: number; end: number } | null = null;
  const node = () => doc.byId[props.id];

  // What the textarea shows. Annotation (PDF highlight) blocks expose only their
  // highlight text (all metadata hidden); every other block hides just the
  // built-in id::/collapsed:: lines (like OG). Hidden lines are preserved and
  // reattached on commit.
  const isAnnot = () => isAnnotationBlock(node().raw);
  // Annotation blocks hide ALL properties (edit only the highlight text); every
  // other block hides just the built-in id::/collapsed::. One fence-aware splitter.
  const hideFn = () => (isAnnot() ? hideAll : isBuiltinHidden);
  const editorValue = () => splitProps(node().raw, hideFn()).visible;
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
  const commit = (text: string) => {
    // For calc, `text` is the bare expressions the user sees — re-fence it.
    const visible = isCalc() ? wrapCalc(text) : text;
    const next = joinProps(visible, splitProps(node().raw, hideFn()).hidden);
    // No-op commit (focus/blur with no real edit, or text that reconstructs the
    // identical raw): don't mark the page dirty or push undo — avoids churn and
    // can't rewrite the block's bytes.
    if (next === node().raw) return;
    setRaw(props.id, next);
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
    // "Create <typed>" is the DEFAULT (first) item whenever the query isn't an
    // exact existing page. So typing a fresh #tag and pressing Enter makes that
    // tag — even when it prefix- or fuzzy-matches an existing page, which would
    // otherwise be silently selected (e.g. #book completing to a "Books" page).
    // The matches still follow, so arrowing down + Enter completes to an existing
    // page instead. No create option for a blank query or an exact match.
    const q = t.query.trim();
    const pageItem = (name: string): AcItem =>
      t.kind === "page"
        ? { label: name, insert: pageInsert(name) }
        : { label: `#${name}`, insert: tagInsert(name) }; // tag context reads "#name"
    const createItem: AcItem =
      t.kind === "page"
        ? { label: `Create "${q}"`, insert: pageInsert(q) }
        : { label: `Create #${q}`, insert: tagInsert(q) };
    const exact = pages.some((p) => p.name.toLowerCase() === q.toLowerCase());
    const items: AcItem[] =
      !q || exact
        ? pages.map((p) => pageItem(p.name)) // exact/blank → no create option
        : [createItem, ...pages.map((p) => pageItem(p.name))]; // else create leads
    setAcItems(items);
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
        setEditingId(null);
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
        lines[0] = setPriority(lines[0], level);
        const next = lines.join("\n");
        commit(next);
        closeAc();
        const caret = lines[0].length;
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

  onMount(() => {
    const offset = takeCaretFor(props.id) ?? editorValue().length;
    ref.focus();
    const o = Math.min(offset, ref.value.length);
    ref.setSelectionRange(o, o);
    resizeNow();
  });

  let acTimer: ReturnType<typeof setTimeout> | undefined;
  const onInput = () => {
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
    const start = ref.selectionStart;
    const raw = ref.value;
    const atEdge =
      dir > 0
        ? !raw.slice(start).includes("\n") && caretAtLastRow(ref, start)
        : !raw.slice(0, start).includes("\n") && caretAtFirstRow(ref, start);
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
      const ll = listLineAt(ref.value, ref.selectionStart);
      if (ll) { nudgeListItem(ll, +2); return true; }
      commit(ref.value); indentBlock(props.id, ref.selectionStart); return true;
    },
    "editor/outdent": (e) => {
      e.preventDefault();
      const ll = listLineAt(ref.value, ref.selectionStart);
      if (ll && ll.indent.length > 0) { nudgeListItem(ll, -2); return true; }
      commit(ref.value); outdentBlock(props.id, ref.selectionStart); return true;
    },
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

    // Configurable editor commands → one dispatch through the handler table
    // (runEditorCmd) instead of ~20 sequential matchesCommand checks. A handler
    // returns false to fall through — select-block does this off the block edge
    // so the textarea extends the selection by a wrapped line.
    const cmd = editorCommandFor(e);
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

    if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
      // In a calc block, Enter adds a new expression line (stays in the block) —
      // let the textarea insert the newline natively, like OG.
      if (isCalc()) return;
      e.preventDefault();
      // In-block list: Enter on a `+`/`*`/ordered list line CONTINUES the list
      // (new item below, same marker/indent; a checkbox item starts a fresh `[ ]`)
      // instead of splitting the block. To exit, Backspace the empty item down to a
      // blank line, then Enter on that non-list line makes a new bullet.
      const ll = !isAnnot() ? listLineAt(raw, start) : null;
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
      // In-block list: Backspace at the head of a list item's text removes the
      // marker (turns it into a blank/plain line) — the way to exit the list.
      const ll = listLineAt(raw, start);
      if (ll && start === ll.lineStart + ll.prefixLen) {
        e.preventDefault();
        applyEdit({ text: raw.slice(0, ll.lineStart) + raw.slice(start), start: ll.lineStart, end: ll.lineStart });
        return;
      }
      if (start === 0) {
        // Never merge a highlight or calc block away (their structure must stay).
        if (isAnnot() || isCalc()) return;
        commit(raw);
        if (mergeWithPrev(props.id)) e.preventDefault();
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
          startEditing(prev, doc.byId[prev].raw.length);
        }
      }
    } else if (e.key === "ArrowDown" && !e.shiftKey) {
      const after = raw.slice(start);
      if (!after.includes("\n") && caretAtLastRow(ref, start)) {
        const next = nextVisible(props.id);
        if (next) {
          e.preventDefault();
          startEditing(next, 0);
        }
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      selectBlock(props.id); // exit editing into block-selection mode
    }
  };

  const onBlur = () => {
    commit(ref.value);
    // A block-move reorder blurs us momentarily — stay in edit mode (the move
    // handler refocuses and restores the caret).
    if (isBlockMoving()) return;
    // The whole window lost focus (switched to another app/window): stay in edit
    // mode and remember the caret so onWindowFocus can resume exactly here. An
    // in-window blur (clicking elsewhere, Escape) keeps document focus, so it
    // still exits editing as before.
    if (!document.hasFocus()) {
      savedSel = { start: ref.selectionStart, end: ref.selectionEnd };
      return;
    }
    // Only clear if no other block grabbed editing focus.
    if (editingId() === props.id) setEditingId(null);
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
    // Single-line/no text: maybe an image on the OS clipboard.
    void (async () => {
      const saved = await backend().pasteImage();
      if (!saved) return;
      const md = assetMarkdown(saved);
      const start = ref.selectionStart;
      const newRaw = ref.value.slice(0, start) + md + ref.value.slice(ref.selectionEnd);
      commit(newRaw);
      const pos = start + md.length;
      queueMicrotask(() => {
        ref.value = newRaw;
        ref.setSelectionRange(pos, pos);
        autosize();
      });
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
        spellcheck={false}
        value={isCalc() ? (calcLive() ?? "") : editorValue()}
        onInput={onInput}
        onKeyDown={onKeyDown}
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
      <Show when={ac() && acItems().length > 0}>
        <div class="autocomplete" ref={acListRef}>
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
      </Show>
    </div>
  );
}

import { Show, Switch, Match, For, createMemo, createSignal, createUniqueId, createEffect, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import {
  detectTrigger,
  applyCompletion,
  pageInsert,
  tagInsert,
  filterCommands,
  type Trigger,
} from "../editor/autocomplete";
import {
  doc,
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
  moveItem,
  selectBlock,
  moveSelection,
  isSelected,
  persistentBlockRef,
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
import { blockView, isPropertyLine } from "../render/block";
import { InlineText } from "../render/inline";
import { BodyContent } from "../render/body";
import { QueryMacro, EmbedMacro } from "./Macro";
import { openPdf, workflow, zoomInto, openContextMenu, openDatePicker, openBlockInSidebar, graphMeta } from "../ui";
import { matchesCommand } from "../keybindings";
import { HL_COLOR_BG, HL_COLOR_SOLID } from "../pdf";
import { cycleMarkerSmart } from "../editor/repeat";

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

// A PDF highlight (annotation) block — its raw is generated metadata, so we
// never let the user drop into raw edit mode (clicking jumps to the PDF).
function isAnnotationBlock(raw: string): boolean {
  return /^\s*ls-type::\s*annotation\s*$/m.test(raw);
}

// For a PDF highlight (annotation) block, resolve the PDF filename from the
// owning hls__ page's `file-path::` property.
function pdfFileForPage(pageName: string): string | null {
  const p = doc.pages.find((x) => x.name === pageName);
  const m = p?.preBlock ? /file-path::\s*(\S+)/.exec(p.preBlock) : null;
  return m ? m[1].split("/").pop() ?? null : null;
}

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
      if (ok) moveBlock(id, tgt.parent, siblingIndex(ind.id) + (ind.before ? 0 : 1));
    }
    setDragId(null);
    setDropInd(null);
    setTimeout(() => (dragMoved = false), 0);
  };
  document.addEventListener("mousemove", onMove);
  document.addEventListener("mouseup", onUp);
}

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
            title="Click to zoom; shift-click to open in sidebar; drag to move"
            onMouseDown={(e) => {
              if (e.button === 0) beginDrag(props.id, e);
            }}
            onClick={(e) => {
              e.stopPropagation();
              if (dragMoved) return; // was a drag, not a click
              if (e.shiftKey) openBlockInSidebar(persistentBlockRef(props.id));
              else zoomInto(props.id);
            }}
          >
            <span class="bullet" />
          </span>
        </div>

        <div
          class="block-content-wrapper"
          onClick={() => {
            // Click anywhere in the row (not on a link) starts editing — and
            // claims the editor for THIS instance.
            if (!editing()) startEditing(props.id, doc.byId[props.id].raw.length, instanceId);
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

function Rendered(props: { id: string; owner?: string }): JSX.Element {
  const node = () => doc.byId[props.id];
  const view = createMemo(() => blockView(node().raw));

  const macro = createMemo(() => detectMacro(view().lines));

  // PDF highlight (annotation) blocks: a colored, clickable swatch of text that
  // opens the PDF at the highlight's page. Notes go in child blocks.
  const annotation = createMemo(() => {
    const props = view().properties;
    if (!props.some(([k, v]) => k === "ls-type" && v === "annotation")) return null;
    const color = props.find(([k]) => k === "hl-color")?.[1] ?? "yellow";
    const hlPage = Number(props.find(([k]) => k === "hl-page")?.[1] ?? "1");
    return { color, hlPage };
  });

  const openHighlightPdf = (e: MouseEvent) => {
    e.stopPropagation();
    const a = annotation();
    const file = pdfFileForPage(node().page);
    if (a && file) openPdf(file, file, a.hlPage);
  };

  // Click edits the block. For annotation blocks the editor shows only the
  // highlight text (metadata stays hidden); the colored prefix still jumps to
  // the PDF.
  const onClick = (e: MouseEvent) => {
    e.stopPropagation();
    startEditing(props.id, node().raw.length, props.owner ?? null);
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
    <Show when={annotation()} fallback={<BodyContent lines={view().lines} blockId={props.id} />}>
      <span class="pdf-annotation-line">
        <span class="hl-prefix" onClick={openHighlightPdf} title="Open in PDF (P{annotation()!.hlPage})">
          <span
            class="hl-dot"
            style={{ background: HL_COLOR_SOLID[annotation()!.color] ?? HL_COLOR_SOLID.yellow }}
          />
          <strong class="hl-page-badge">P{annotation()!.hlPage}</strong>
        </span>{" "}
        <span
          class="hl-text"
          style={{ background: HL_COLOR_BG[annotation()!.color] ?? HL_COLOR_BG.yellow }}
        >
          <InlineText text={view().lines[0]} />
        </span>
      </span>
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
  insert?: string;
  caret?: number;
  action?: import("../editor/autocomplete").CommandAction;
  templateNodes?: import("../types").BlockDto[];
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
async function getTemplates(): Promise<import("../types").TemplateDto[]> {
  if (templateCache) return templateCache;
  try {
    templateCache = await backend().listTemplates();
  } catch {
    templateCache = [];
  }
  return templateCache;
}
/** Replace Logseq dynamic template vars: <% today/yesterday/tomorrow/time %>. */
function applyTemplateVars(raw: string): string {
  return raw.replace(/<%\s*(today|yesterday|tomorrow|time|current time)\s*%>/gi, (_m, kw) => {
    const k = String(kw).toLowerCase();
    if (k === "time" || k === "current time") return timeStamp();
    const d = new Date();
    if (k === "yesterday") d.setDate(d.getDate() - 1);
    if (k === "tomorrow") d.setDate(d.getDate() + 1);
    return `[[${todayJournalName(d)}]]`;
  });
}
function templateToOutline(b: import("../types").BlockDto): { raw: string; children: any[] } {
  return { raw: applyTemplateVars(b.raw), children: b.children.map(templateToOutline) };
}
// Markdown for a freshly saved asset: images embed inline, everything else
// (PDFs included) becomes a link — a .pdf link renders as a clickable chip that
// opens the PDF pane.
function assetMarkdown(name: string): string {
  return /\.(png|jpe?g|gif|webp|svg|bmp|avif)$/i.test(name)
    ? `![](../assets/${name})`
    : `[${name}](../assets/${name})`;
}

// Split a block's raw into editable text vs. trailing `key:: value` property
// lines. For annotation blocks we edit only the text (the highlight) and keep
// the metadata (hl-page/hl-color/ls-type/id) hidden but preserved.
function splitProps(raw: string): { text: string; props: string } {
  const text: string[] = [];
  const props: string[] = [];
  for (const l of raw.split("\n")) (isPropertyLine(l) ? props : text).push(l);
  return { text: text.join("\n"), props: props.join("\n") };
}
function joinProps(text: string, props: string): string {
  return props ? `${text}\n${props}` : text;
}

function Editor(props: { id: string }): JSX.Element {
  let ref!: HTMLTextAreaElement;
  const node = () => doc.byId[props.id];

  // Annotation (PDF highlight) blocks expose only their highlight text in the
  // editor; the metadata properties stay hidden but are reattached on commit.
  const isAnnot = () => isAnnotationBlock(node().raw);
  const editorValue = () => (isAnnot() ? splitProps(node().raw).text : node().raw);
  const commit = (text: string) =>
    setRaw(props.id, isAnnot() ? joinProps(text, splitProps(node().raw).props) : text);

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
      const cmds: AcItem[] = filterCommands(t.query).map((c) => ({
        label: c.label,
        insert: c.insert,
        caret: c.caret,
        action: c.action,
      }));
      const tmpls = await getTemplates();
      const q = t.query.toLowerCase();
      const showAll = q.length > 0 && "template".startsWith(q); // typing /t…/template
      const tItems: AcItem[] = tmpls
        .filter((tp) => showAll || (q.length > 0 && tp.name.toLowerCase().includes(q)))
        .map((tp) => ({ label: `Template: ${tp.name}`, templateNodes: tp.blocks }));
      const cur = ac();
      if (!cur || cur.start !== t.start) return; // trigger changed while awaiting
      setAcItems([...cmds, ...tItems]);
      return;
    }
    const pages = await backend().quickSwitch(t.query, 8);
    const cur = ac();
    if (!cur || cur.start !== t.start) return; // trigger changed while awaiting
    const items: AcItem[] = pages.map((p) =>
      t.kind === "page"
        ? { label: p.name, insert: pageInsert(p.name) }
        : { label: p.name, insert: tagInsert(p.name) }
    );
    if (t.query.trim() && !pages.some((p) => p.name.toLowerCase() === t.query.toLowerCase())) {
      items.push(
        t.kind === "page"
          ? { label: `Create "${t.query}"`, insert: pageInsert(t.query) }
          : { label: `Create #${t.query}`, insert: tagInsert(t.query) }
      );
    }
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
      const saved = await backend().importAsset(path);
      const md = assetMarkdown(saved);
      const pos = ref.selectionStart;
      const nr = ref.value.slice(0, pos) + md + ref.value.slice(pos);
      setRaw(props.id, nr);
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
    if (item.templateNodes) {
      // Drop the "/name" trigger text, then insert the template's blocks (with
      // dynamic vars resolved). If the host block is now empty, replace it.
      const r = applyCompletion(ref.value, t.start, t.end, "");
      commit(r.raw);
      closeAc();
      const nodes = item.templateNodes.map(templateToOutline);
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

  const autosize = () => {
    ref.style.height = "auto";
    ref.style.height = `${ref.scrollHeight}px`;
  };

  onMount(() => {
    const offset = takeCaretFor(props.id) ?? editorValue().length;
    ref.focus();
    const o = Math.min(offset, ref.value.length);
    ref.setSelectionRange(o, o);
    autosize();
  });

  const onInput = () => {
    commit(ref.value);
    autosize();
    void updateAutocomplete();
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

    // Inline-format toggles + Emacs-style motions (pure text ops in format.ts).
    if (matchesCommand(e, "editor/bold")) { e.preventDefault(); applyEdit(toggleWrap(raw, start, end, "**")); return; }
    if (matchesCommand(e, "editor/italics")) { e.preventDefault(); applyEdit(toggleWrap(raw, start, end, "*")); return; }
    if (matchesCommand(e, "editor/strike-through")) { e.preventDefault(); applyEdit(toggleWrap(raw, start, end, "~~")); return; }
    if (matchesCommand(e, "editor/highlight")) { e.preventDefault(); applyEdit(toggleWrap(raw, start, end, "==")); return; }
    if (matchesCommand(e, "editor/insert-link")) { e.preventDefault(); applyEdit(insertLink(raw, start, end)); return; }
    if (matchesCommand(e, "editor/clear-block")) { e.preventDefault(); applyEdit({ text: "", start: 0, end: 0 }); return; }
    if (matchesCommand(e, "editor/kill-line-before")) { e.preventDefault(); applyEdit(killLineBefore(raw, start)); return; }
    if (matchesCommand(e, "editor/kill-line-after")) { e.preventDefault(); applyEdit(killLineAfter(raw, start)); return; }
    if (matchesCommand(e, "editor/backward-kill-word")) { e.preventDefault(); applyEdit(killWordBackward(raw, start)); return; }
    if (matchesCommand(e, "editor/forward-kill-word")) { e.preventDefault(); applyEdit(killWordForward(raw, start)); return; }
    if (matchesCommand(e, "editor/backward-word")) { e.preventDefault(); moveCaret(wordBackward(raw, start)); return; }
    if (matchesCommand(e, "editor/forward-word")) { e.preventDefault(); moveCaret(wordForward(raw, start)); return; }

    // Configurable editor shortcuts (resolved against config.edn :shortcuts).
    // Move block up/down: reorder among siblings, keeping edit mode + caret
    // (the DOM reorder can briefly blur the textarea).
    if (matchesCommand(e, "editor/move-block-up") || matchesCommand(e, "editor/move-block-down")) {
      e.preventDefault();
      commit(raw);
      moveItem(props.id, matchesCommand(e, "editor/move-block-down") ? 1 : -1);
      startEditing(props.id, start);
      return;
    }

    // Collapse / expand the current block's children (ctrl+up / ctrl+down).
    if (matchesCommand(e, "editor/collapse")) {
      e.preventDefault();
      setCollapsed(props.id, true);
      return;
    }
    if (matchesCommand(e, "editor/expand")) {
      e.preventDefault();
      setCollapsed(props.id, false);
      return;
    }

    // Select block up/down: at the block's first/last line, start a block
    // selection (current block + neighbour); the global handler extends it.
    if (matchesCommand(e, "editor/select-block-up") || matchesCommand(e, "editor/select-block-down")) {
      const down = matchesCommand(e, "editor/select-block-down");
      const atEdge = down ? !raw.slice(start).includes("\n") : !raw.slice(0, start).includes("\n");
      if (atEdge) {
        e.preventDefault();
        commit(raw);
        selectBlock(props.id);
        moveSelection(down ? 1 : -1, true);
        return;
      }
    }

    if (matchesCommand(e, "editor/cycle-todo")) {
      e.preventDefault();
      const { raw: newRaw, delta } = cycleMarkerSmart(raw, workflow());
      commit(newRaw);
      const pos = Math.max(0, start + delta);
      queueMicrotask(() => {
        ref.value = newRaw;
        ref.setSelectionRange(pos, pos);
        autosize();
      });
    } else if (matchesCommand(e, "editor/indent")) {
      e.preventDefault();
      commit(raw);
      indentBlock(props.id, start);
    } else if (matchesCommand(e, "editor/outdent")) {
      e.preventDefault();
      commit(raw);
      outdentBlock(props.id, start);
    } else if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
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
    } else if (e.key === "Backspace" && start === 0 && end === 0) {
      // Never merge a highlight block away (its metadata must stay intact).
      if (isAnnot()) return;
      commit(raw);
      if (mergeWithPrev(props.id)) e.preventDefault();
    } else if (e.key === "ArrowUp") {
      const before = raw.slice(0, start);
      if (!before.includes("\n")) {
        const prev = prevVisible(props.id);
        if (prev) {
          e.preventDefault();
          startEditing(prev, doc.byId[prev].raw.length);
        }
      }
    } else if (e.key === "ArrowDown") {
      const after = raw.slice(start);
      if (!after.includes("\n")) {
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
    // Only clear if no other block grabbed editing focus.
    if (editingId() === props.id) setEditingId(null);
  };

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
      const md = `![](../assets/${saved})`;
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
    <div class="editor-wrap">
      <textarea
        ref={ref}
        class="block-editor"
        spellcheck={false}
        value={editorValue()}
        onInput={onInput}
        onKeyDown={onKeyDown}
        onBlur={onBlur}
        onPaste={onPaste}
        onSelect={updateSel}
        onMouseUp={updateSel}
        rows={1}
      />
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
                {item.label}
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

import { Show, Switch, Match, For, createMemo, createSignal, onMount, type JSX } from "solid-js";
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
  startEditing,
  setRaw,
  setEditingId,
  splitBlock,
  indentBlock,
  outdentBlock,
  mergeWithPrev,
  toggleCollapse,
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
} from "../store";
import { parseOutline } from "../editor/outline";
import { blockView } from "../render/block";
import { InlineText } from "../render/inline";
import { BodyContent } from "../render/body";
import { QueryMacro, EmbedMacro } from "./Macro";
import { openPdf, workflow, zoomInto, openContextMenu } from "../ui";
import { matchesCommand } from "../keybindings";
import { HL_COLOR_BG, HL_COLOR_SOLID } from "../pdf";
import { cycleMarker } from "../editor/marker";

// Detect a block whose entire body is a single {{query}} / {{embed}} macro.
function detectMacro(lines: string[]): { kind: "query" | "embed"; inner: string } | null {
  const text = lines.join("\n").trim();
  const m = /^\{\{(query|embed)\b([\s\S]*)\}\}$/.exec(text);
  if (!m) return null;
  return { kind: m[1] as "query" | "embed", inner: `${m[1]}${m[2]}` };
}

// Internal/metadata properties hidden from the rendered properties area.
const INTERNAL_PROPS = new Set(["id", "collapsed", "hl-page", "hl-color", "hl-type", "ls-type"]);

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
  const editing = () => editingId() === props.id;
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
            title="Click to zoom; drag to move"
            onMouseDown={(e) => {
              if (e.button === 0) beginDrag(props.id, e);
            }}
            onClick={(e) => {
              e.stopPropagation();
              if (dragMoved) return; // was a drag, not a click
              zoomInto(props.id);
            }}
          >
            <span class="bullet" />
          </span>
        </div>

        <div
          class="block-content-wrapper"
          onClick={() => {
            // Click anywhere in the row (not on a link) starts editing.
            if (!editing()) startEditing(props.id, doc.byId[props.id].raw.length);
          }}
        >
          <Show when={editing()} fallback={<Rendered id={props.id} />}>
            <Editor id={props.id} />
          </Show>
        </div>
      </div>

      <Show when={hasChildren() && !collapsed()}>
        <div class="block-children-container">
          <div class="block-children-left-border" />
          <div class="block-children">
            <For each={node().children}>{(cid) => <Block id={cid} />}</For>
          </div>
        </div>
      </Show>
    </div>
  );
}

function Rendered(props: { id: string }): JSX.Element {
  const node = () => doc.byId[props.id];
  const view = createMemo(() => blockView(node().raw));

  const onClick = (e: MouseEvent) => {
    // Links/tags handle their own clicks (stopPropagation). Otherwise edit.
    startEditing(props.id, node().raw.length);
    e.stopPropagation();
  };

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

  const displayProps = () => view().properties.filter(([k]) => !INTERNAL_PROPS.has(k));

  const body = (
    <Show when={annotation()} fallback={<BodyContent lines={view().lines} />}>
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
              <QueryMacro body={macro()!.inner} />
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
      classList={{ done: view().done, [`heading h${view().headingLevel ?? ""}`]: view().headingLevel != null }}
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
        <span class="date-chip scheduled" title="Scheduled">
          🗓 {view().scheduled}
        </span>
      </Show>
      <Show when={view().deadline}>
        <span class="date-chip deadline" title="Deadline">
          ⏰ {view().deadline}
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
  const { raw } = cycleMarker(doc.byId[id].raw, workflow());
  setRaw(id, raw);
}

interface AcItem {
  label: string;
  insert: string;
  caret?: number;
}

function Editor(props: { id: string }): JSX.Element {
  let ref!: HTMLTextAreaElement;
  const node = () => doc.byId[props.id];

  const [ac, setAc] = createSignal<Trigger | null>(null);
  const [acItems, setAcItems] = createSignal<AcItem[]>([]);
  const [acIndex, setAcIndex] = createSignal(0);

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
      setAcItems(filterCommands(t.query).map((c) => ({ label: c.label, insert: c.insert, caret: c.caret })));
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

  const selectAc = (item: AcItem) => {
    const t = ac();
    if (!t) return;
    const r = applyCompletion(ref.value, t.start, t.end, item.insert, item.caret);
    setRaw(props.id, r.raw);
    closeAc();
    queueMicrotask(() => {
      ref.value = r.raw;
      ref.setSelectionRange(r.caret, r.caret);
      ref.focus();
      autosize();
    });
  };

  const autosize = () => {
    ref.style.height = "auto";
    ref.style.height = `${ref.scrollHeight}px`;
  };

  onMount(() => {
    const offset = takeCaretFor(props.id) ?? node().raw.length;
    ref.focus();
    const o = Math.min(offset, ref.value.length);
    ref.setSelectionRange(o, o);
    autosize();
  });

  const onInput = () => {
    setRaw(props.id, ref.value);
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

    // Configurable editor shortcuts (resolved against config.edn :shortcuts).
    // Move block up/down: reorder among siblings, keeping edit mode + caret
    // (the DOM reorder can briefly blur the textarea).
    if (matchesCommand(e, "editor/move-block-up") || matchesCommand(e, "editor/move-block-down")) {
      e.preventDefault();
      setRaw(props.id, raw);
      moveItem(props.id, matchesCommand(e, "editor/move-block-down") ? 1 : -1);
      startEditing(props.id, start);
      return;
    }

    // Select block up/down: at the block's first/last line, start a block
    // selection (current block + neighbour); the global handler extends it.
    if (matchesCommand(e, "editor/select-block-up") || matchesCommand(e, "editor/select-block-down")) {
      const down = matchesCommand(e, "editor/select-block-down");
      const atEdge = down ? !raw.slice(start).includes("\n") : !raw.slice(0, start).includes("\n");
      if (atEdge) {
        e.preventDefault();
        setRaw(props.id, raw);
        selectBlock(props.id);
        moveSelection(down ? 1 : -1, true);
        return;
      }
    }

    if (matchesCommand(e, "editor/cycle-todo")) {
      e.preventDefault();
      const { raw: newRaw, delta } = cycleMarker(raw, workflow());
      setRaw(props.id, newRaw);
      const pos = Math.max(0, start + delta);
      queueMicrotask(() => {
        ref.value = newRaw;
        ref.setSelectionRange(pos, pos);
        autosize();
      });
    } else if (matchesCommand(e, "editor/indent")) {
      e.preventDefault();
      setRaw(props.id, raw);
      indentBlock(props.id, start);
    } else if (matchesCommand(e, "editor/outdent")) {
      e.preventDefault();
      setRaw(props.id, raw);
      outdentBlock(props.id, start);
    } else if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      setRaw(props.id, raw); // flush current text
      splitBlock(props.id, start);
    } else if (e.key === "Backspace" && start === 0 && end === 0) {
      setRaw(props.id, raw);
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
    setRaw(props.id, ref.value);
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
      setRaw(props.id, ref.value);
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
      setRaw(props.id, newRaw);
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
        value={node().raw}
        onInput={onInput}
        onKeyDown={onKeyDown}
        onBlur={onBlur}
        onPaste={onPaste}
        rows={1}
      />
      <Show when={ac() && acItems().length > 0}>
        <div class="autocomplete">
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

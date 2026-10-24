// Nonshipping experiment. No CM syntax parser is configured: lsdoc supplies all spans.
import { createEffect, createSignal, For, onCleanup, onMount, type JSX } from "solid-js";
import { EditorSelection, EditorState, Prec, StateField } from "@codemirror/state";
import { Decoration, EditorView, keymap, WidgetType, type DecorationSet } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, redo, undo } from "@codemirror/commands";
import { blockWritable, mergeWithPrev, nextVisible, node, pageByName, prevVisible, setRaw, splitBlock, type OutlineScope } from "../document";
import { endEdit, noteSurfaceFocused, startEditing, takeCaretFor } from "../editorController";
import { BUILTIN_HIDE, joinProps, splitProps } from "../editor/properties";
import { previewSpans } from "../editor/cm6Preview";
import { detectTrigger, pageInsert, refCompletionEnd } from "../editor/autocomplete";
import { allPageNames } from "../pageIndex";
import "./cm6-spike.css";

export function cm6SpikeEnabled(): boolean {
  return (import.meta.env.DEV || import.meta.env.VITE_CM6_SPIKE === "1")
    && typeof location !== "undefined"
    && (new URLSearchParams(location.search).get("cm6") === "1" || sessionStorage.getItem("tine-spike-cm6") === "1");
}

class Chip extends WidgetType {
  constructor(readonly label: string, readonly from: number) { super(); }
  eq(other: Chip) { return other.label === this.label && other.from === this.from; }
  toDOM(view: EditorView) {
    const el = view.dom.ownerDocument.createElement("span");
    el.className = "cm6-chip";
    el.textContent = this.label;
    el.addEventListener("mousedown", event => {
      event.preventDefault();
      view.dispatch({ selection: { anchor: this.from }, scrollIntoView: true });
      view.focus();
    });
    return el;
  }
  ignoreEvent() { return true; }
}

export function Cm6BlockEditor(props: { id: string; surface: string; scope: OutlineScope | null }): JSX.Element {
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  let removeHistoryKeys: (() => void) | undefined;
  let alive = true;
  const [candidates, setCandidates] = createSignal<string[]>([]);
  const shownNode = node(props.id);
  const format = pageByName(shownNode.page)?.format === "org" ? "org" : "md";
  const current = () => alive && node(props.id) === shownNode && blockWritable(props.id);
  const visible = () => splitProps(node(props.id).raw, BUILTIN_HIDE, format).visible;
  const commit = (text: string) => {
    if (!current() || text === visible()) return;
    const raw = node(props.id).raw;
    setRaw(props.id, joinProps(text, splitProps(raw, BUILTIN_HIDE, format).hidden, format, raw));
  };
  const navSurface = () => props.surface.startsWith("ref:") || props.surface.startsWith("embed:") ? props.surface : null;
  const editSurface = () => props.surface.startsWith("embed:") ? props.surface : null;
  const buildDecorations = (state: EditorState): DecorationSet => {
    const ranges = previewSpans(state.doc.toString(), format).map(span => {
      // Reveal at either boundary and whenever any selection intersects source.
      const reveal = state.selection.ranges.some(r => r.from <= span.to && r.to >= span.from);
      const deco = (span.kind === "task" || span.kind === "page") && !reveal
        ? Decoration.replace({ widget: new Chip(span.label!, span.from) })
        : Decoration.mark({ class: `cm6-${span.kind}` });
      return deco.range(span.from, span.to);
    });
    return Decoration.set(ranges, true);
  };
  const decorations = StateField.define<DecorationSet>({
    create: buildDecorations,
    update: (value, tr) => tr.docChanged || tr.selection ? buildDecorations(tr.state) : value,
    provide: field => EditorView.decorations.from(field),
  });
  const vertical = (forward: boolean) => (v: EditorView) => {
    const selection = v.state.selection.main;
    if (!selection.empty || v.compositionStarted) return false;
    // CM6 already resolves wrapped visual-row boundaries. Comparing caret
    // glyph rectangles to line boxes failed with ordinary WebKit font leading.
    const boundary = v.moveToLineBoundary(selection, forward).head;
    const atEdge = forward ? boundary === v.state.doc.length : boundary === 0;
    if (!atEdge) return false;
    const target = forward ? nextVisible(props.id, props.scope) : prevVisible(props.id, props.scope);
    if (!target) return false;
    const rowStart = v.moveToLineBoundary(selection, false).head;
    startEditing(target, { col: selection.head - rowStart, edge: forward ? "first" : "last" }, null, navSurface());
    return true;
  };
  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({ doc: visible(), extensions: [
        history(), decorations, EditorView.lineWrapping,
        EditorView.contentAttributes.of({ "data-cm6-editor": props.id, "aria-label": "" }),
        Prec.highest(keymap.of([
          { key: "Mod-Shift-z", run: redo },
          { key: "Enter", run: v => {
            if (!current() || v.compositionStarted) return false;
            const s = v.state.selection.main;
            if (!s.empty) v.dispatch({ changes: { from: s.from, to: s.to } });
            commit(v.state.doc.toString());
            const zoom = props.scope?.forceExpandedRoot === props.id;
            splitBlock(props.id, s.from, zoom, zoom, editSurface());
            return true;
          } },
          { key: "Backspace", run: v => !v.compositionStarted && current() && v.state.selection.main.empty
            && v.state.selection.main.head === 0 && mergeWithPrev(props.id, props.scope?.navOnly ? null : props.scope, editSurface()) },
          { key: "ArrowUp", run: vertical(false) },
          { key: "ArrowDown", run: vertical(true) },
          { key: "Escape", run: v => { if (v.compositionStarted) return false; endEdit("select-block"); return true; } },
        ])),
        keymap.of([...historyKeymap, ...defaultKeymap]),
        EditorView.domEventHandlers({
          focus: () => { noteSurfaceFocused(props.surface); return false; },
          blur: (_event, v) => { if (alive && !v.hasFocus) endEdit("blur"); return false; },
          beforeinput: () => !current(),
        }),
        EditorView.updateListener.of(update => {
          if (update.docChanged) commit(update.state.doc.toString());
          if (update.docChanged || update.selectionSet) {
            const s = update.state.selection.main;
            const trigger = detectTrigger(update.state.doc.toString(), s.head, null, format);
            host.dataset.pageAutocomplete = trigger?.kind === "page" ? "open" : "closed";
            // Crude local inventory only: no lease, aliases, ranking or keyboard picker.
            setCandidates(trigger?.kind === "page" ? allPageNames().filter(name => name.toLowerCase().includes(trigger.query.toLowerCase())).slice(0, 5) : []);
          }
        }),
      ] }),
    });
    // An inspectable view is useful to the experiment harness, never a production API.
    (host as HTMLDivElement & { cm6View?: EditorView }).cm6View = view;
    // Tine's global document-history capture precedes target keymaps. Own
    // default in-block history chords at the window boundary for this spike.
    const historyKeys = (event: KeyboardEvent) => {
      if (!view?.contentDOM.contains(event.target as Node)) return;
      // composing becomes true only after the first DOM change; the public
      // compositionStarted also covers preedit before that first change.
      // Leave the native IME default action intact, but keep application and
      // CM keymaps out of both modern and legacy-229 composition key events.
      if (event.isComposing || event.keyCode === 229 || view.compositionStarted) {
        event.stopImmediatePropagation();
        return;
      }
      if (!(event.ctrlKey || event.metaKey) || event.altKey) return;
      const key = event.key.toLowerCase();
      if (key !== "z" && key !== "y") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (current()) (key === "y" || event.shiftKey ? redo : undo)(view);
    };
    host.ownerDocument.defaultView!.addEventListener("keydown", historyKeys, true);
    removeHistoryKeys = () => host.ownerDocument.defaultView!.removeEventListener("keydown", historyKeys, true);
    const want = takeCaretFor(props.id);
    let anchor = view.state.doc.length, head = anchor;
    if (typeof want === "number") anchor = head = want;
    else if (want && "start" in want) { anchor = want.start; head = want.end; }
    else if (want && "edge" in want) {
      const edge = want.edge === "first" ? 0 : view.state.doc.length;
      const row = view.moveToLineBoundary(EditorSelection.cursor(edge), false).head;
      const end = view.moveToLineBoundary(EditorSelection.cursor(edge), true).head;
      anchor = head = Math.min(row + want.col, end);
    }
    view.dispatch({ selection: { anchor: Math.min(anchor, view.state.doc.length), head: Math.min(head, view.state.doc.length) }, scrollIntoView: true });
    view.focus();
  });
  createEffect(() => {
    const text = visible();
    if (view && text !== view.state.doc.toString() && !view.compositionStarted) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
    }
  });
  onCleanup(() => { alive = false; removeHistoryKeys?.(); view?.destroy(); });
  const complete = (name: string) => {
    if (!view || !current()) return;
    const raw = view.state.doc.toString();
    const trigger = detectTrigger(raw, view.state.selection.main.head, null, format);
    if (trigger?.kind !== "page") return;
    const insert = pageInsert(name);
    view.dispatch({ changes: { from: trigger.start, to: refCompletionEnd(raw, trigger.end, insert), insert }, selection: { anchor: trigger.start + insert.length } });
    view.focus();
  };
  return <div class="cm6-spike" ref={host}><div class="cm6-completions"><For each={candidates()}>{name =>
    <button onMouseDown={event => { event.preventDefault(); complete(name); }}>{name}</button>
  }</For></div></div>;
}

// Nonshipping experiment. No CM syntax parser is configured: lsdoc supplies all spans.
import { createEffect, createSignal, For, Show, onCleanup, onMount, type JSX } from "solid-js";
import { EditorSelection, EditorState, Prec, StateField } from "@codemirror/state";
import { Decoration, EditorView, keymap, WidgetType, type DecorationSet } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, redo, undo } from "@codemirror/commands";
import { blockWritable, mergeWithPrev, nextVisible, node, pageByName, prevVisible, setRaw, splitBlock, type OutlineScope } from "../document";
import { endEdit, noteSurfaceFocused, startEditing, takeCaretFor } from "../editorController";
import { BUILTIN_HIDE, joinProps, splitProps } from "../editor/properties";
import { previewSpans } from "../editor/cm6Preview";
import { detectTrigger, pageInsert, refCompletionEnd } from "../editor/autocomplete";
import { allPageNames } from "../pageIndex";
import { registerFocusedEditorCommandBridge } from "../editorCommandBridge";
import { SELECTION_ACTIONS, type SelectionAction } from "../editor/selectionActions";
import { touchPrimary } from "../inputCapability";
import "./cm6-spike.css";

export { cm6SpikeEnabled } from "../editor/cm6Spike";

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
  let removeCommandBridge: (() => void) | undefined;
  let alive = true;
  const [candidates, setCandidates] = createSignal<string[]>([]);
  const [hasSelection, setHasSelection] = createSignal(false);
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
  const structuralEnter = (v: EditorView) => {
    if (!current() || v.compositionStarted) return false;
    const s = v.state.selection.main;
    if (!s.empty) v.dispatch({ changes: { from: s.from, to: s.to } });
    commit(v.state.doc.toString());
    const zoom = props.scope?.forceExpandedRoot === props.id;
    splitBlock(props.id, s.from, zoom, zoom, editSurface());
    return true;
  };
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
          { key: "Enter", run: structuralEnter },
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
          setHasSelection(!update.state.selection.main.empty);
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
      // CM6 6.43.15 skips keymaps for 100ms after compositionend on
      // desktop WebKit. IBus Hangul has already committed and forwards a
      // normal Enter here, so own the split before CM's suppression can
      // leave WebKit's native line break in the block. Consumed IME/229
      // keys stay above this boundary; modified Enter keeps its keymap.
      if (!event.defaultPrevented && event.key === "Enter" && !event.shiftKey
        && !event.ctrlKey && !event.metaKey && !event.altKey && structuralEnter(view)) {
        event.preventDefault();
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
    removeCommandBridge = registerFocusedEditorCommandBridge({
      blockId: props.id,
      blur: () => view?.contentDOM.blur(),
      textSelection: () => {
        const s = view?.state.selection.main;
        return view && s && !s.empty ? { text: view.state.doc.toString(), start: s.from, end: s.to } : null;
      },
      dispatch: command => {
        if (!view || !current() || view.compositionStarted) return false;
        // Only the spike's source insertions are adapted here. Structural,
        // media, picker, slash-menu and global-history parity remains deferred.
        const s = view.state.selection.main;
        const selected = view.state.sliceDoc(s.from, s.to);
        let insert: string, caret: number;
        if (command === "editor/soft-newline") { insert = "\n"; caret = s.from + 1; }
        else if (command === "editor/insert-page-ref" || command === "editor/insert-block-ref") {
          const page = command === "editor/insert-page-ref";
          insert = (page ? "[[" : "((") + selected + (page ? "]]" : "))");
          caret = s.from + 2 + selected.length;
        } else return false;
        view.dispatch({ changes: { from: s.from, to: s.to, insert }, selection: { anchor: caret }, scrollIntoView: true });
        view.focus();
        return true;
      },
    });
  });
  createEffect(() => {
    const text = visible();
    if (view && text !== view.state.doc.toString() && !view.compositionStarted) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
    }
  });
  onCleanup(() => { alive = false; removeCommandBridge?.(); removeHistoryKeys?.(); view?.destroy(); });
  const formatSelection = (action: SelectionAction) => {
    if (!view || !current() || view.compositionStarted) return;
    const s = view.state.selection.main;
    const edit = action.apply(view.state.doc.toString(), s.from, s.to, format);
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: edit.text }, selection: { anchor: edit.start, head: edit.end }, scrollIntoView: true });
    view.focus();
  };
  const complete = (name: string) => {
    if (!view || !current()) return;
    const raw = view.state.doc.toString();
    const trigger = detectTrigger(raw, view.state.selection.main.head, null, format);
    if (trigger?.kind !== "page") return;
    const insert = pageInsert(name);
    view.dispatch({ changes: { from: trigger.start, to: refCompletionEnd(raw, trigger.end, insert), insert }, selection: { anchor: trigger.start + insert.length } });
    view.focus();
  };
  return <div class="cm6-spike" ref={host}>
    <Show when={hasSelection()}>
      <div class="sel-toolbar" classList={{ "sel-toolbar-mobile": touchPrimary() }} data-mobile-selection-toolbar={touchPrimary() ? "" : undefined} onPointerDown={event => event.preventDefault()}>
        <For each={SELECTION_ACTIONS}>{action => <button type="button" classList={{ "sel-action-page-link": action.id === "page-link" }} title={action.title} aria-label={action.title} data-selection-action={action.id} onClick={() => formatSelection(action)}>{action.label}</button>}</For>
      </div>
    </Show>
    <div class="cm6-completions"><For each={candidates()}>{name =>
    <button onMouseDown={event => { event.preventDefault(); complete(name); }}>{name}</button>
  }</For></div></div>;
}

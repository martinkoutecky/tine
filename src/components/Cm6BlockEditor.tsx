// Experimental editor. No CM syntax parser is configured: lsdoc supplies all spans.
import { createEffect, createSignal, For, Show, onCleanup, onMount, useContext, type JSX } from "solid-js";
import { EditorSelection, EditorState, Prec, StateField } from "@codemirror/state";
import { Decoration, EditorView, keymap, WidgetType, type DecorationSet } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, redo, undo } from "@codemirror/commands";
import { blockWritable, isBlockMoving, node, pageByName, setRaw, type OutlineScope } from "../document";
import { clearFocusSurface, editingId, endEdit, focusSurfaceFor, noteSurfaceFocused, takeCaretFor } from "../editorController";
import { BUILTIN_HIDE, SHEET_CELL_HIDE, joinProps, splitProps } from "../editor/properties";
import { previewSpans } from "../editor/cm6Preview";
import { registerFocusedEditorCommandBridge } from "../editorCommandBridge";
import { SELECTION_ACTIONS, type SelectionAction } from "../editor/selectionActions";
import { touchPrimary } from "../inputCapability";
import { cm6Structure } from "../editor/cm6Structure";
import { CollapseSurfaceContext, EmbedNavExitContext } from "./Block";
import { marginNavigation } from "./blockComments";
import { SheetCellContext } from "../sheet/context";
import { cm6SheetKeys } from "../editor/cm6SheetKeys";
import { isPermittedTabGesture } from "../keybindings";
import "./cm6-editor.css";

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
  let editorHost!: HTMLDivElement;
  let view: EditorView | undefined;
  let removeHistoryKeys: (() => void) | undefined;
  let removeCommandBridge: (() => void) | undefined;
  let alive = true;
  const [hasSelection, setHasSelection] = createSignal(false);
  const shownNode = node(props.id);
  const format = pageByName(shownNode.page)?.format === "org" ? "org" : "md";
  const current = () => alive && node(props.id) === shownNode && blockWritable(props.id);
  const cell = useContext(SheetCellContext);
  const hiddenProperties = cell ? SHEET_CELL_HIDE : BUILTIN_HIDE;
  const visible = () => splitProps(node(props.id).raw, hiddenProperties, format).visible;
  const commit = (text: string) => {
    if (!current() || text === visible()) return;
    const raw = node(props.id).raw;
    setRaw(props.id, joinProps(text, splitProps(raw, hiddenProperties, format).hidden, format, raw));
  };
  const embedExit = useContext(EmbedNavExitContext);
  const structure = cm6Structure({ ...props, format, current, commit, embedExit,
    collapse: useContext(CollapseSurfaceContext),
    ...marginNavigation(props.id, props.scope, props.surface, () => format),
  });
  const sheet = cell ? cm6SheetKeys(props.id, cell, commit) : null;
  const buildDecorations = (state: EditorState): DecorationSet => {
    const ranges = previewSpans(state.doc.toString(), format).flatMap(span => {
      // Reveal at either boundary and whenever any selection intersects source.
      const reveal = state.selection.ranges.some(r => r.from <= span.to && r.to >= span.from);
      const mark = Decoration.mark({ class: `cm6-${span.kind}` });
      if (reveal || span.kind === "task") return [mark.range(span.from, span.to)];
      if (span.kind === "page") {
        // Hide only the brackets: the name stays real text, so a click lands where it was made.
        const src = state.doc.sliceString(span.from, span.to);
        if (src.startsWith("[[") && src.endsWith("]]") && span.to - span.from > 4) {
          const hidden = Decoration.replace({});
          return [hidden.range(span.from, span.from + 2), mark.range(span.from + 2, span.to - 2), hidden.range(span.to - 2, span.to)];
        }
        return [Decoration.replace({ widget: new Chip(span.label!, span.from) }).range(span.from, span.to)];
      }
      return [mark.range(span.from, span.to)];
    });
    return Decoration.set(ranges, true);
  };
  const decorations = StateField.define<DecorationSet>({
    create: buildDecorations,
    update: (value, tr) => tr.docChanged || tr.selection ? buildDecorations(tr.state) : value,
    provide: field => EditorView.decorations.from(field),
  });
  onMount(() => {
    view = new EditorView({
      parent: editorHost,
      state: EditorState.create({ doc: visible(), extensions: [
        history(), decorations, EditorView.lineWrapping,
        EditorView.editorAttributes.of({ "data-block-editor": "cm6" }),
        EditorView.contentAttributes.of({ "data-cm6-editor": props.id, "aria-label": "" }),
        Prec.highest(keymap.of([
          { key: "Mod-Shift-z", run: redo },
        ])),
        keymap.of([...historyKeymap, ...defaultKeymap]),
        EditorView.domEventHandlers({
          focus: () => { noteSurfaceFocused(props.surface); return false; },
          blur: (event, v) => {
            // Reorders can blur a retained editor; the structural gesture owns
            // its focus restoration. Palette actions still need embed context.
            if (isBlockMoving() || embedExit && (event.relatedTarget as HTMLElement | null)?.closest?.(".switcher")) return false;
            if (current() && editingId() === props.id && !v.hasFocus) endEdit("blur");
            return false;
          },
          beforeinput: () => !current(),
        }),
        EditorView.updateListener.of(update => {
          setHasSelection(!update.state.selection.main.empty);
          if (update.docChanged) commit(update.state.doc.toString());
        }),
      ] }),
    });
    // Tine's global document-history capture precedes target keymaps. Own
    // default in-block history chords at the window boundary while the experimental editor is active.
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
    const structuralKeys = (event: KeyboardEvent) => {
      if (!view || !view.contentDOM.contains(event.target as Node) || !current()
        || event.isComposing || event.keyCode === 229 || view.compositionStarted) return;
      // Global capture (including transient/suspension ownership) runs first.
      // Tab is pre-cancelled there specifically for the editor to own.
      if (event.defaultPrevented && !isPermittedTabGesture(event)) return;
      // Run before CM's target handler: desktop WebKit suppresses its keymaps
      // just after compositionend, including an already committed plain Enter.
      if (sheet?.keydown(event, view) || structure.keydown(event, view)) {
        event.preventDefault();
        event.stopImmediatePropagation();
      }
    };
    host.ownerDocument.defaultView!.addEventListener("keydown", historyKeys, true);
    host.addEventListener("keydown", structuralKeys, true);
    removeHistoryKeys = () => {
      host.ownerDocument.defaultView!.removeEventListener("keydown", historyKeys, true);
      host.removeEventListener("keydown", structuralKeys, true);
    };
    const want = takeCaretFor(props.id);
    let anchor = view.state.doc.length, head = anchor;
    if (typeof want === "number") anchor = head = want;
    else if (want && "start" in want) {
      anchor = want.direction === "backward" ? want.end : want.start;
      head = want.direction === "backward" ? want.start : want.end;
    }
    else if (want && "edge" in want) {
      const edge = want.edge === "first" ? 0 : view.state.doc.length;
      const row = view.moveToLineBoundary(EditorSelection.cursor(edge), false).head;
      const end = view.moveToLineBoundary(EditorSelection.cursor(edge), true).head;
      anchor = head = Math.min(row + want.col, end);
    }
    view.dispatch({ selection: { anchor: Math.min(anchor, view.state.doc.length), head: Math.min(head, view.state.doc.length) }, scrollIntoView: true });
    const focusSurface = focusSurfaceFor(props.id);
    if (!focusSurface || focusSurface === props.surface) {
      view.focus();
      queueMicrotask(() => clearFocusSurface(props.id));
    } else {
      queueMicrotask(() => {
        if (!view || !current() || !view.dom.isConnected || editingId() !== props.id) return;
        const active = host.ownerDocument.activeElement;
        if (!active?.closest('textarea.block-editor, [data-block-editor="cm6"]')) {
          view.focus();
          clearFocusSurface(props.id);
        }
      });
    }
    removeCommandBridge = registerFocusedEditorCommandBridge({
      blockId: props.id,
      blur: () => view?.contentDOM.blur(),
      replaceText: (text, container) => {
        if (!view || !current() || view.compositionStarted || !container.contains(view.contentDOM)) return false;
        view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, selection: { anchor: text.length }, userEvent: "input" });
        return true;
      },
      textSelection: () => {
        const s = view?.state.selection.main;
        return view && s && !s.empty ? { text: view.state.doc.toString(), start: s.from, end: s.to } : null;
      },
      dispatch: command => {
        if (!view || !current() || view.compositionStarted) return false;
        if (sheet?.blocks(command)) return true;
        if (structure.command(view, command)) return true;
        // Media, pickers, slash menu and global-history parity are later ports.
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
    if (!current()) return;
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
  return <div class="cm6-block-editor" ref={host}>
    <div ref={editorHost} />
    <Show when={hasSelection()}>
      <div class="sel-toolbar" classList={{ "sel-toolbar-mobile": touchPrimary() }} data-mobile-selection-toolbar={touchPrimary() ? "" : undefined} onPointerDown={event => event.preventDefault()}>
        <For each={SELECTION_ACTIONS}>{action => <button type="button" classList={{ "sel-action-page-link": action.id === "page-link" }} title={action.title} aria-label={action.title} data-selection-action={action.id} onClick={() => formatSelection(action)}>{action.label}</button>}</For>
      </div>
    </Show>
  </div>;
}

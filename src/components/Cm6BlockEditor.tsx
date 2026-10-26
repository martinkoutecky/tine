// Experimental editor. No CM syntax parser is configured: lsdoc supplies all spans.
import { createEffect, onCleanup, onMount, useContext, type JSX } from "solid-js";
import { EditorSelection, EditorState, StateField } from "@codemirror/state";
import { Decoration, EditorView, keymap, WidgetType, type DecorationSet } from "@codemirror/view";
import { defaultKeymap } from "@codemirror/commands";
import { blockWritable, isBlockMoving, node, pageByName, setRaw, type OutlineScope } from "../document";
import { clearFocusSurface, editingId, editingOwner, endEdit, focusSurfaceFor, noteSurfaceFocused, registerHistoryEditorTarget, takeCaretFor, takeHistoryEditorSelectionFor } from "../editorController";
import { BUILTIN_HIDE, SHEET_CELL_HIDE } from "../editor/properties";
import { propertyEditorSession } from "../editor/propertySession";
import { previewSpans } from "../editor/cm6Preview";
import { registerFocusedEditorCommandBridge } from "../editorCommandBridge";
import { cm6Formatting } from "../editor/cm6Formatting";
import { cm6Structure } from "../editor/cm6Structure";
import { CollapseSurfaceContext, EmbedNavExitContext } from "./Block";
import { marginNavigation } from "./blockComments";
import { SheetCellContext } from "../sheet/context";
import { cm6SheetKeys } from "../editor/cm6SheetKeys";
import { cm6Clipboard } from "../editor/cm6Clipboard";
import { cm6Completions } from "../editor/cm6Completions";
import { isPermittedTabGesture } from "../keybindings";
import { isNodeValue } from "../windowRealm";
import { nearestScrollableY } from "./blockParts";
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

export function Cm6BlockEditor(props: { id: string; surface: string; scope: OutlineScope | null; propertySession?: ReturnType<typeof propertyEditorSession> }): JSX.Element {
  let host!: HTMLDivElement;
  let editorHost!: HTMLDivElement;
  let view: EditorView | undefined;
  let removeKeys: (() => void) | undefined;
  let removeCommandBridge: (() => void) | undefined;
  let alive = true;
  const propertySession = props.propertySession ?? propertyEditorSession();
  propertySession.begin();
  const shownNode = node(props.id);
  const format = pageByName(shownNode.page)?.format === "org" ? "org" : "md";
  const current = () => alive && node(props.id) === shownNode && blockWritable(props.id);
  const cell = useContext(SheetCellContext);
  const hiddenProperties = cell ? SHEET_CELL_HIDE : BUILTIN_HIDE;
  const parts = () => propertySession.split(node(props.id).raw, hiddenProperties, format);
  const visible = () => parts().visible;
  let rawBeforeCommit = shownNode.raw;
  const commit = (text: string) => {
    if (!current() || text === visible()) return;
    const raw = node(props.id).raw;
    rawBeforeCommit = raw;
    setRaw(props.id, propertySession.join(text, parts().hidden, raw, hiddenProperties, format));
  };
  const embedExit = useContext(EmbedNavExitContext);
  const structure = cm6Structure({ ...props, format, current, commit, embedExit,
    collapse: useContext(CollapseSurfaceContext),
    ...marginNavigation(props.id, props.scope, props.surface, () => format),
  });
  const sheet = cell ? cm6SheetKeys(props.id, cell, commit) : null;
  const clipboard = cm6Clipboard({ id: props.id, format, current, commit, sheet: !!cell, hiddenProperties });
  const completion = cm6Completions({ ...props, format, current, sheet: !!cell, view: () => view, visible, rawBeforeCommit: () => rawBeforeCommit });
  const formatting = cm6Formatting({ ...props, format, current, commit, view: () => view,
    closeCompletion: completion.close, refreshCompletion: completion.refresh });
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
        decorations, EditorView.lineWrapping, completion.inputSelection,
        EditorView.editorAttributes.of({ "data-block-editor": "cm6" }),
        EditorView.contentAttributes.of({ "data-cm6-editor": props.id, "aria-label": "" }),
        keymap.of(defaultKeymap),
        EditorView.domEventHandlers({
          focus: () => { noteSurfaceFocused(props.surface); return false; },
          blur: (event, v) => {
            // Reorders can blur a retained editor; the structural gesture owns
            // its focus restoration. Palette actions still need embed context.
            if (isBlockMoving() || (event.relatedTarget as HTMLElement | null)?.closest?.(".date-picker")
              || embedExit && (event.relatedTarget as HTMLElement | null)?.closest?.(".switcher")) return false;
            if (current() && editingId() === props.id && !v.hasFocus) endEdit("blur");
            return false;
          },
          beforeinput: () => !current(),
          input: event => { completion.input(event as InputEvent); return false; },
          compositionstart: () => { completion.close(); return false; },
          compositionend: () => { queueMicrotask(completion.refresh); return false; },
          paste: (event, v) => clipboard.paste(event, v),
          // CM defaults to copying/cutting a whole line without a selection.
          // Tine's empty-selection Mod+C is the durable block-ref command;
          // other empty-selection clipboard events keep the source unchanged.
          copy: (_event, v) => v.state.selection.main.empty,
          cut: (_event, v) => !current() || v.state.selection.main.empty,
          keyup: event => { if (event.key.toLowerCase() === "v") clipboard.clearRaw(); return false; },
        }),
        EditorView.updateListener.of(update => {
          formatting.changed(update);
          if (update.docChanged) commit(update.state.doc.toString());
          completion.changed(update);
        }),
      ] }),
    });
    completion.mount();
    onCleanup(registerHistoryEditorTarget({ blockId: props.id, owner: editingOwner(), surface: props.surface,
      selection: () => { const s = view!.state.selection.main; return { start: s.from, end: s.to }; },
      viewport: () => ({ editor: view!.contentDOM, scroller: nearestScrollableY(view!.contentDOM) }),
      focused: () => !!view?.hasFocus }));
    // Keep application/CM commands out of native IME preedit. Ordinary history
    // belongs to the document, just like the mobile toolbar and textarea.
    const compositionKeys = (event: KeyboardEvent) => {
      if (!isNodeValue(event.target) || !view?.contentDOM.contains(event.target)) return;
      // composing becomes true only after the first DOM change; the public
      // compositionStarted also covers preedit before that first change.
      // Leave the native IME default action intact, but keep application and
      // CM keymaps out of both modern and legacy-229 composition key events.
      if (event.isComposing || event.keyCode === 229 || view.compositionStarted) {
        event.stopImmediatePropagation();
        return;
      }
    };
    const structuralKeys = (event: KeyboardEvent) => {
      if (!view || !view.contentDOM.contains(event.target as Node) || !current()
        || event.isComposing || event.keyCode === 229 || view.compositionStarted) return;
      // Global capture (including transient/suspension ownership) runs first.
      // Tab is pre-cancelled there specifically for the editor to own.
      if (event.defaultPrevented && !isPermittedTabGesture(event)) return;
      // Run before CM's target handler: desktop WebKit suppresses its keymaps
      // just after compositionend, including an already committed plain Enter.
      if (completion.keydown(event) || clipboard.keydown(event, view) || sheet?.keydown(event, view) || formatting.keydown(event) || structure.keydown(event, view)) {
        event.preventDefault();
        event.stopImmediatePropagation();
      }
    };
    host.ownerDocument.defaultView!.addEventListener("keydown", compositionKeys, true);
    host.addEventListener("keydown", structuralKeys, true);
    removeKeys = () => {
      host.ownerDocument.defaultView!.removeEventListener("keydown", compositionKeys, true);
      host.removeEventListener("keydown", structuralKeys, true);
    };
    const want = takeCaretFor(props.id);
    const historySelection = takeHistoryEditorSelectionFor(props.id, props.surface);
    let anchor = view.state.doc.length, head = anchor;
    if (historySelection) { anchor = historySelection.start; head = historySelection.end; }
    else if (typeof want === "number") anchor = head = want;
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
        if (completion.command(command)) return true;
        if (clipboard.command(view, command)) return true;
        if (formatting.command(command)) return true;
        if (structure.command(view, command)) return true;
        // Media parity remains a later port.
        return false;
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
  onCleanup(() => { alive = false; clipboard.clearRaw(); removeCommandBridge?.(); removeKeys?.(); view?.destroy(); });
  return <div class="cm6-block-editor" ref={host}>
    <div ref={editorHost} />
    <completion.Popup />
    <formatting.Toolbar />
  </div>;
}

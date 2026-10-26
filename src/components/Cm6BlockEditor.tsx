// Experimental editor. No CM syntax parser is configured: lsdoc supplies all spans.
import { createEffect, createMemo, createSignal, For, Show, onCleanup, onMount, useContext, type JSX } from "solid-js";
import { Annotation, Compartment, EditorSelection, EditorState, StateEffect, StateField } from "@codemirror/state";
import { Decoration, EditorView, keymap, WidgetType, type DecorationSet } from "@codemirror/view";
import { defaultKeymap } from "@codemirror/commands";
import { blockWritable, depthOf, isBlockMoving, node, pageByName, setRaw, type OutlineScope } from "../document";
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
import { editorAssets } from "../editor/editorAssets";
import { cm6Completions } from "../editor/cm6Completions";
import { cm6Typing } from "../editor/cm6Typing";
import { offsetThroughRewrite } from "../editor/caretThroughRewrite";
import { isPermittedTabGesture } from "../keybindings";
import { isNodeValue } from "../windowRealm";
import { nearestScrollableY } from "./blockParts";
import { createCodeBodyEditor } from "../editor/codeBodyEditor";
import { codeFenceOnly } from "../editor/codeFence";
import { codeFences } from "../editor/fences";
import { calcSource, evalCalc, serializeCalcExitCommit } from "../editor/calc";
import { codeWrapping } from "../codeDisplay";
import { LineGutter } from "../render/LineGutter";
import { AstBody } from "../render/body";
import { CopyButton } from "../render/inline";
import { t as translate } from "../i18n";
import { touchPrimary } from "../inputCapability";
import { openContextMenu } from "../ui";
import { renderedClickOffset } from "./blockGestures";
import { forbidsEditEntry } from "../editor/editTargets";
import { effectiveHeadingLevel } from "../render/facets";
import { parserReady } from "../render/parse";
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
  // A directly mounted editor can outlive its node during graph replacement.
  // Its projections remain readable until disposal; current() gates all writes.
  const source = () => node(props.id)?.raw ?? shownNode.raw;
  const parts = () => propertySession.split(source(), hiddenProperties, format);
  const visible = () => parts().visible;
  const [isCalc, setCalc] = createSignal(calcSource(visible()) !== null);
  const [pickingLanguage, setPickingLanguage] = createSignal(false);
  const codeView = createCodeBodyEditor(visible, () => format, () => !cell && !isCalc() && !pickingLanguage(), source);
  const code = codeView.shown;
  const literal = () => isCalc() || code() !== null;
  const shown = () => isCalc() ? calcSource(visible()) ?? visible() : code()?.body ?? visible();
  const codeEditing = () => code() !== null || codeFenceOnly(visible(), format) !== null;
  const calcRows = createMemo(() => isCalc() ? evalCalc(shown()) : []);
  const mixedContext = createMemo(() => {
    const p = code();
    if (!p || !codeView.mixed()) return null;
    const raw = visible(), fence = codeFences(raw, format).find(f => f.openEnd === p.open.length);
    return fence ? { before: raw.slice(0, fence.start), after: raw.slice(fence.end) } : null;
  });
  const projection = Annotation.define<boolean>();
  const wrapping = new Compartment();
  const editability = new Compartment();
  const refreshPreview = StateEffect.define<null>();
  let updating = false;
  let rawBeforeCommit = shownNode.raw;
  const commit = (text: string) => {
    if (!current() || text === shown()) return;
    const raw = node(props.id).raw;
    rawBeforeCommit = raw;
    const next = isCalc() ? null : codeView.join(text);
    setRaw(props.id, next ?? propertySession.join(isCalc() ? serializeCalcExitCommit(text, visible()) : text, parts().hidden, raw, hiddenProperties, format));
  };
  // Projection transactions change the local coordinate space only. Never
  // publish those replacements as edits or run completion/typing transforms.
  const showText = (v: EditorView, text: string, anchor: number, head = anchor) => {
    const limit = (n: number) => Math.max(0, Math.min(text.length, n));
    v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: text },
      selection: { anchor: limit(anchor), head: limit(head) }, annotations: projection.of(true) });
  };
  const syncView = () => {
    if (!view || updating || !current() || typing.active()) return;
    const text = shown(), before = view.state.doc.toString(), s = view.state.selection.main;
    if (text !== before) showText(view, text, offsetThroughRewrite(before, text, s.anchor), offsetThroughRewrite(before, text, s.head));
    view.contentDOM.classList.toggle("code-edit", codeEditing());
  };
  const enterBody = () => {
    if (!view || literal() || pickingLanguage()) return;
    const s = view.state.selection.main;
    if (!s.empty) return;
    codeView.enter(s.head);
    const p = code();
    if (p) showText(view, p.body, s.anchor - p.open.length, s.head - p.open.length);
  };
  const editMixedContext = (event: MouseEvent, side: "before" | "after") => {
    const context = mixedContext();
    if (!view || !context || event.button !== 0 || forbidsEditEntry(event)) return;
    const raw = context[side];
    const offset = (side === "after" ? visible().length - raw.length : 0)
      + (renderedClickOffset(event.currentTarget as HTMLElement, raw, format, event) ?? raw.length);
    event.preventDefault(); event.stopPropagation();
    codeView.enter(offset);
    showText(view, shown(), offset - (code()?.open.length ?? 0));
    view.focus();
  };
  const embedExit = useContext(EmbedNavExitContext);
  const structure = cm6Structure({ ...props, format, current, commit, embedExit, code: () => code() !== null, calc: isCalc,
    collapse: useContext(CollapseSurfaceContext),
    ...marginNavigation(props.id, props.scope, props.surface, () => format),
  });
  const sheet = cell ? cm6SheetKeys(props.id, cell, commit) : null;
  const assets = editorAssets({ id: props.id, element: () => view!.contentDOM, mounted: current,
    insert: markup => {
      if (!view) return;
      view.dispatch(view.state.replaceSelection(markup), { userEvent: "input.paste", scrollIntoView: true });
      view.focus();
    },
  });
  const clipboard = cm6Clipboard({ id: props.id, format, current, commit, literal, assets, sheet: !!cell, hiddenProperties });
  const completion = cm6Completions({ ...props, format, current, literal, sheet: !!cell, view: () => view, visible, rawBeforeCommit: () => rawBeforeCommit,
    assets,
    languagePicker: setPickingLanguage,
    completeRegion: (text, caret) => {
      if (!view) return;
      const calc = calcSource(text);
      if (calc !== null) setCalc(true);
      const p = code();
      if (calc !== null || p) showText(view, shown(), caret - (calc !== null ? text.indexOf("\n") + 1 : p!.open.length));
    },
  });
  const typing = cm6Typing({ id: props.id, format, current, view: () => view, commit,
    literal, scaffold: completion.scaffold, refresh: completion.refresh, pauseCompletion: completion.pause });
  const formatting = cm6Formatting({ ...props, format, current, commit, view: () => view,
    closeCompletion: completion.close, refreshCompletion: completion.refresh });
  const buildDecorations = (state: EditorState): DecorationSet => {
    if (!parserReady() || literal()) return Decoration.none;
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
    update: (value, tr) => tr.docChanged || tr.selection || tr.effects.some(effect => effect.is(refreshPreview)) ? buildDecorations(tr.state) : value,
    provide: field => EditorView.decorations.from(field),
  });
  onMount(() => {
    const want = takeCaretFor(props.id);
    const historySelection = takeHistoryEditorSelectionFor(props.id, props.surface);
    // Numeric targets use visible-source coordinates. History uses body
    // coordinates for whole code and source coordinates for mixed occurrences.
    if (!code() && !isCalc()) {
      if (historySelection) codeView.enter(historySelection.start, historySelection.end);
      else if (typeof want === "number") codeView.enter(want);
      else if (want && "start" in want) codeView.enter(want.start, want.end);
    }
    view = new EditorView({
      parent: editorHost,
      dispatchTransactions: (transactions, v) => {
        updating = true;
        try { v.update(transactions); } finally { updating = false; }
        syncView();
        if (transactions.some(tr => tr.isUserEvent("select"))) enterBody();
      },
      state: EditorState.create({ doc: shown(), extensions: [
        decorations, wrapping.of(!isCalc() && (!codeEditing() || codeWrapping()) ? EditorView.lineWrapping : []),
        editability.of([EditorState.readOnly.of(!current()), EditorView.editable.of(current())]), completion.inputSelection, typing.filter,
        EditorView.editorAttributes.of({ "data-block-editor": "cm6" }),
        EditorView.contentAttributes.of({ "data-cm6-editor": props.id, "aria-label": "" }),
        keymap.of(defaultKeymap),
        EditorView.domEventHandlers({
          focus: () => { noteSurfaceFocused(props.surface); return false; },
          blur: (event, v) => {
            // Reorders can blur a retained editor; the structural gesture owns
            // its focus restoration. Palette actions still need embed context.
            if (assets.holdsBlur() || isBlockMoving() || (event.relatedTarget as HTMLElement | null)?.closest?.(".date-picker")
              || embedExit && (event.relatedTarget as HTMLElement | null)?.closest?.(".switcher")) return false;
            const leave = () => { if (current() && editingId() === props.id && !v.hasFocus) endEdit("blur"); };
            if (typing.active()) queueMicrotask(leave);
            else leave();
            return false;
          },
          beforeinput: event => { if ((event as InputEvent).isComposing) typing.begin(); return !current(); },
          input: event => { if (typing.input(event as InputEvent) && !typing.active()) completion.input(event as InputEvent); return false; },
          compositionstart: () => { typing.begin(); return false; },
          compositionend: () => { typing.end(); return false; },
          paste: (event, v) => clipboard.paste(event, v),
          contextmenu: (event, v) => {
            if (!codeEditing() || touchPrimary() || !v.state.selection.main.empty) return false;
            event.preventDefault(); event.stopPropagation();
            openContextMenu(event.clientX, event.clientY, props.id);
            return true;
          },
          // CM defaults to copying/cutting a whole line without a selection.
          // Tine's empty-selection Mod+C is the durable block-ref command;
          // other empty-selection clipboard events keep the source unchanged.
          copy: (_event, v) => v.state.selection.main.empty,
          cut: (_event, v) => !current() || v.state.selection.main.empty,
          keyup: event => { if (event.key.toLowerCase() === "v") clipboard.clearRaw(); return false; },
        }),
        EditorView.updateListener.of(update => {
          formatting.changed(update);
          if (update.transactions.every(tr => tr.annotation(projection))) return;
          if (typing.changed(update) && !typing.active()) completion.changed(update);
        }),
      ] }),
    });
    completion.mount();
    onCleanup(registerHistoryEditorTarget({ blockId: props.id, owner: editingOwner(), surface: props.surface,
      selection: () => { const s = view!.state.selection.main, offset = codeView.mixed() ? code()!.open.length : 0; return { start: s.from + offset, end: s.to + offset }; },
      viewport: () => ({ editor: view!.contentDOM, scroller: nearestScrollableY(view!.contentDOM) }),
      focused: () => !!view?.hasFocus }));
    // Keep application/CM commands out of native IME preedit. Ordinary history
    // belongs to the document, just like the mobile toolbar and textarea.
    const compositionKeys = (event: KeyboardEvent) => {
      if (!isNodeValue(event.target) || !view?.contentDOM.contains(event.target)) return;
      if (!event.isComposing && event.keyCode !== 229) typing.beforeKey();
      // composing becomes true only after the first DOM change; the public
      // compositionStarted also covers preedit before that first change.
      // Leave the native IME default action intact, but keep application and
      // CM keymaps out of both modern and legacy-229 composition key events.
      if (event.isComposing || event.keyCode === 229 || typing.active()) {
        event.stopImmediatePropagation();
        return;
      }
    };
    const structuralKeys = (event: KeyboardEvent) => {
      if (!view || !view.contentDOM.contains(event.target as Node) || !current()
        || event.isComposing || event.keyCode === 229 || typing.active()) return;
      // Global capture (including transient/suspension ownership) runs first.
      // Tab is pre-cancelled there specifically for the editor to own.
      if (event.defaultPrevented && !isPermittedTabGesture(event)) return;
      // Run before CM's target handler: desktop WebKit suppresses its keymaps
      // just after compositionend, including an already committed plain Enter.
      const s = view.state.selection.main;
      const leave = codeView.leaveRange(event, view.state.doc.toString(), s.from, s.to);
      if (leave) showText(view, visible(), leave.anchor, leave.head);
      if (leave || completion.keydown(event) || clipboard.keydown(event, view) || sheet?.keydown(event, view) || formatting.keydown(event) || structure.keydown(event, view)) {
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
    let anchor = view.state.doc.length, head = anchor;
    if (historySelection) { anchor = historySelection.start; head = historySelection.end; }
    else if (typeof want === "number") anchor = head = Math.max(0, want - (code()?.open.length ?? 0));
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
    if (codeView.mixed() && (historySelection || want && typeof want === "object" && "start" in want)) {
      anchor -= code()!.open.length; head -= code()!.open.length;
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
        if (!view || !current() || typing.active() || !container.contains(view.contentDOM)) return false;
        view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, selection: { anchor: text.length }, userEvent: "input" });
        return true;
      },
      textSelection: () => {
        const s = view?.state.selection.main;
        return view && s && !s.empty ? { text: view.state.doc.toString(), start: s.from, end: s.to } : null;
      },
      dispatch: command => {
        typing.beforeKey();
        if (!view || !current() || typing.active()) return false;
        if (sheet?.blocks(command)) return true;
        if (completion.command(command)) return true;
        if (clipboard.command(view, command)) return true;
        if (formatting.command(command)) return true;
        if (structure.command(view, command)) return true;
        if (command === "editor/upload-asset") { void assets.uploadAsset(); return true; }
        if (command === "editor/capture-photo") { void assets.capturePhotoCmd(); return true; }
        if (command === "editor/voice-memo") { void assets.voiceMemoToggle(); return true; }
        return false;
      },
    });
  });
  createEffect(() => {
    if (!current()) return;
    shown(); codeEditing();
    syncView();
  });
  createEffect(() => {
    const wrap = !isCalc() && (!codeEditing() || codeWrapping());
    view?.dispatch({ effects: wrapping.reconfigure(wrap ? EditorView.lineWrapping : []) });
  });
  createEffect(() => {
    const writable = current();
    view?.dispatch({ effects: editability.reconfigure([EditorState.readOnly.of(!writable), EditorView.editable.of(writable)]) });
  });
  createEffect(() => {
    if (parserReady()) view?.dispatch({ effects: refreshPreview.of(null) });
  });
  onCleanup(() => { alive = false; typing.dispose(); clipboard.clearRaw(); removeCommandBridge?.(); removeKeys?.(); view?.destroy(); });
  return <div class="cm6-block-editor editor-wrap" classList={{ "calc-wrap": isCalc(), "code-wrapping": codeEditing() && codeWrapping() }} ref={host}>
    <Show when={mixedContext()?.before}>{before => <div class="block-content mixed-code-before" onMouseDown={event => editMixedContext(event, "before")}><AstBody raw={before()} format={format} headingLevel={effectiveHeadingLevel(propertySession.facets(source(), format), depthOf(props.id))} /></div>}</Show>
    <div class="code-editor-region" classList={{ "mixed-code-region": !!mixedContext() }}>
      <Show when={codeEditing()}>
        <span class="code-language">{(code() ?? codeFenceOnly(visible(), format))?.lang.toLowerCase()}</span>
        <LineGutter lines={shown().split("\n")} code />
      </Show>
      <Show when={isCalc()}><LineGutter lines={calcRows().map(row => row.input)} /></Show>
      <div ref={editorHost} />
    </div>
    <Show when={mixedContext()?.after}>{after => <div class="block-content mixed-code-after" onMouseDown={event => editMixedContext(event, "after")}><AstBody raw={after()} format={format} /></div>}</Show>
    <Show when={isCalc()}><div class="calc-results"><For each={calcRows()}>{row =>
      <div class="calc-out" classList={{ "calc-error": !!row.error }}>{row.output ?? ""}
        <Show when={row.output !== null && !row.error}><CopyButton text={row.output!} title={translate("editor-block-copy-result")} class="calc-copy" /></Show>
      </div>
    }</For></div></Show>
    <completion.Popup />
    <formatting.Toolbar />
  </div>;
}

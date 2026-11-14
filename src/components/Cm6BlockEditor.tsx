// Experimental editor. lsdoc owns structural spans; highlight.js supplies code tokens.
import { createEffect, createMemo, createSignal, For, Show, getOwner, onCleanup, onMount, untrack, useContext, type JSX } from "solid-js";
import { Annotation, Compartment, EditorSelection, EditorState, StateEffect, StateField } from "@codemirror/state";
import { Decoration, EditorView, keymap, type DecorationSet } from "@codemirror/view";
import { defaultKeymap } from "@codemirror/commands";
import { blockWritable, depthOf, finishPageHeaderEdit, isBlockMoving, node, pageByName, rawToWrite, setRaw, type OutlineScope, type PendingEdit } from "../document";
import { captureEditIntent, editingId, editingOwner, endEdit, noteSurfaceFocused, registerHistoryEditorTarget, takeCaretFor, takeHistoryEditorSelectionFor, type CaretPos } from "../editorController";
import { focusMountedEditor } from "../editor/surfaceFocus";
import { cm6CaretReveal } from "../editor/cm6CaretReveal";
import { cm6EmptyCaret } from "../editor/cm6EmptyCaret";
import { cm6CodeHighlight, refreshCodeHighlight } from "../editor/cm6CodeHighlight";
import { SHEET_CELL_HIDE } from "../editor/properties";
import { propertyEditorSession } from "../editor/propertySession";
import { editorCommitRecovery } from "../editor/commitRecovery";
import { commitBlockEditorExit, editorMarkerBaseline } from "../editor/commitExit";
import { editorWindowFocus } from "../editor/windowFocus";
import { captureEditorScrollAnchor } from "../editor/scrollAnchor";
import { annotationInfo } from "../editor/annotation";
import { inPageFindPreservesEditorBlur } from "../inpageFind";
import { concealedSpans, previewSpans } from "../editor/cm6Preview";
import { PreviewWidget } from "../editor/cm6PreviewWidget";
import { registerFocusedEditorCommandBridge } from "../editorCommandBridge";
import { cm6Formatting } from "../editor/cm6Formatting";
import { cm6Structure } from "../editor/cm6Structure";
import { commentKey } from "../editor/commentKeys";
import { CaptureCtx, CollapseSurfaceContext, EmbedNavExitContext } from "./Block";
import { captureKeyIntent } from "../editor/captureKeys";
import { cm6CommandHandlers } from "../editor/cm6Commands";
import { commentEditorHide, marginNavigation } from "./blockComments";
import { SheetCellContext } from "../sheet/context";
import { cellOwner } from "../sheet/selection";
import { cm6SheetKeys } from "../editor/cm6SheetKeys";
import { editsLikeOutline, gridNestedBlock, gridOutlineScope, runGridOutline } from "../sheet/gridEditMove";
import { cm6Clipboard } from "../editor/cm6Clipboard";
import { editorAssets } from "../editor/editorAssets";
import { cm6Completions } from "../editor/cm6Completions";
import { cm6Typing } from "../editor/cm6Typing";
import { offsetThroughRewrite } from "../editor/caretThroughRewrite";
import { editorCommandFor, isPermittedTabGesture, isWebviewReloadChord } from "../keybindings";
import { spellcheckEnabled } from "../spellcheckSettings";
import { documentHasFocus, isNodeValue } from "../windowRealm";
import { nearestScrollableY } from "./blockParts";
import { createCodeBodyEditor } from "../editor/codeBodyEditor";
import { codeBodyProjection, codeFenceOnly } from "../editor/codeFence";
import { codeFences } from "../editor/fences";
import { calcSource, evalCalc, serializeCalcExitCommit } from "../editor/calc";
import { codeWrapping } from "../codeDisplay";
import { codeGutterStyle, LineGutter } from "../render/LineGutter";
import { AstBody } from "../render/body";
import { CopyButton } from "../render/inline";
import { t as translate } from "../i18n";
import { touchPrimary } from "../inputCapability";
import { openContextMenu } from "../ui";
import { renderedClickOffset } from "./blockGestures";
import { forbidsEditEntry } from "../editor/editTargets";
import { effectiveHeadingLevel } from "../render/facets";
import { parserReady } from "../render/parse";
import { blockDirAttr } from "../render/textDirection";
import "./cm6-editor.css";

export function Cm6BlockEditor(props: { id: string; surface: string; scope: OutlineScope | null; propertySession?: ReturnType<typeof propertyEditorSession> }): JSX.Element {
  const owner = getOwner();
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
  const capture = useContext(CaptureCtx);
  const hiddenProperties = cell ? SHEET_CELL_HIDE
    : commentEditorHide(shownNode, propertySession.facets(shownNode.raw, format).properties, format, node);
  // A directly mounted editor can outlive its node during graph replacement.
  // Its projections remain readable until disposal; current() gates all writes.
  const source = () => node(props.id)?.raw ?? shownNode.raw;
  const parts = () => propertySession.split(source(), hiddenProperties, format);
  const visible = () => parts().visible;
  const [isCalc, setCalc] = createSignal(calcSource(visible()) !== null);
  const [pickingLanguage, setPickingLanguage] = createSignal(false);
  const codeView = createCodeBodyEditor(visible, () => format, () => !cell && !isCalc() && !pickingLanguage(), source, true);
  const code = codeView.shown;
  const literal = () => isCalc() || code() !== null;
  const shown = () => isCalc() ? calcSource(visible()) ?? visible() : code()?.body ?? visible();
  const codeEditing = () => code() !== null || codeFenceOnly(visible(), format) !== null;
  const codeLines = createMemo(() => codeEditing() ? shown().split("\n") : []);
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
  const spellcheck = new Compartment();
  const refreshPreview = StateEffect.define<null>();
  let updating = false;
  let rawBeforeCommit = shownNode.raw;
  const markerBaseline = editorMarkerBaseline(shownNode.raw, format);
  const commitRecovery = editorCommitRecovery(() => view?.contentDOM, () => node(props.id) === shownNode,
    () => view?.state.doc.toString() ?? "");
  let pendingScrollAnchor: ReturnType<typeof captureEditorScrollAnchor> | undefined;
  let anchorFrame: number | undefined;
  /** The write `commit(text)` would make, decided but not made: `raw` is the block's text once written
   *  (the clock transition settled, so it runs once), `write` writes it. A refused join throws here,
   *  as in `commit`. A structural gesture that replaces the selection stages the replacement, asks
   *  the kernel about the text it leaves and writes only on an answer. */
  const stage = (text: string): PendingEdit => {
    const unchanged = (): PendingEdit => ({ raw: node(props.id)?.raw ?? shownNode.raw, write() {} });
    if (!current()) return unchanged();
    if (text === shown()) { commitRecovery.applied(); return unchanged(); }
    const raw = node(props.id).raw;
    rawBeforeCommit = raw;
    const next = commitRecovery.join(() => (isCalc() ? null : codeView.join(text))
      ?? propertySession.join(isCalc() ? serializeCalcExitCommit(text, visible()) : text, parts().hidden, raw, hiddenProperties, format), text);
    if (next === raw) return unchanged();
    const settled = rawToWrite(props.id, next, { markerBaseline: markerBaseline(raw) });
    return {
      raw: settled,
      write() {
        // Capture before live mirrors react; one restoration per frame, with the
        // same focus/scroll/gesture ownership as the textarea autosize path.
        if (view && pendingScrollAnchor === undefined) {
          pendingScrollAnchor = captureEditorScrollAnchor(view.contentDOM, nearestScrollableY(view.contentDOM));
          anchorFrame = host.ownerDocument.defaultView!.requestAnimationFrame(() => {
            pendingScrollAnchor?.restore(view?.contentDOM ?? null);
            pendingScrollAnchor = undefined; anchorFrame = undefined;
          });
        }
        setRaw(props.id, settled, { timetracking: false });
      },
    };
  };
  const commit = (text: string) => stage(text).write();
  /** Deleting `from..to` as a staged edit. Its write takes the text out of the view as a projection, so
   *  the typing listener does not commit it a second time; `stage` publishes it once. */
  const deleteSelection = (v: EditorView, from: number, to: number): PendingEdit => {
    const staged = stage(v.state.doc.sliceString(0, from) + v.state.doc.sliceString(to));
    return { raw: staged.raw, write() { v.dispatch({ changes: { from, to }, annotations: projection.of(true) }); staged.write(); } };
  };
  // Projection transactions change the local coordinate space only. Never
  // publish those replacements as edits or run completion/typing transforms.
  const showText = (v: EditorView, text: string, anchor: number, head = anchor) => {
    const limit = (n: number) => Math.max(0, Math.min(text.length, n));
    v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: text },
      selection: { anchor: limit(anchor), head: limit(head) }, annotations: projection.of(true) });
  };
  const syncView = () => {
    if (!view || updating || !current() || typing.active() || commitRecovery.held()) return;
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
  const scope = cell && editsLikeOutline(cell) ? gridOutlineScope(cell) : props.scope;
  const structure = cm6Structure({ ...props, scope, cell, format, current, commit, stage, deleteSelection, embedExit, code: () => code() !== null, calc: isCalc,
    annotation: () => annotationInfo(propertySession.facets(source(), format).properties) !== null,
    collapse: useContext(CollapseSurfaceContext),
    ...marginNavigation(props.id, scope, props.surface, () => format),
  });
  const sheet = cell ? cm6SheetKeys(props.id, cell, commit) : null;
  const assets = editorAssets({ id: props.id, element: () => view!.contentDOM, mounted: current, capture: !!capture,
    insert: markup => {
      if (!view) return;
      view.dispatch(view.state.replaceSelection(markup), { userEvent: "input.paste", scrollIntoView: true });
      view.focus();
    },
  });
  const clipboard = cm6Clipboard({ id: props.id, format, current, commit, stage, literal, assets, cell, sheet: !!cell && !gridNestedBlock(props.id, cell), hiddenProperties });
  const completion = cm6Completions({ ...props, format, current, literal, sheet: !!cell, view: () => view, visible, rawBeforeCommit: () => rawBeforeCommit,
    stageDelete: (from, to) => deleteSelection(view!, from, to),
    annotation: () => annotationInfo(propertySession.facets(source(), format).properties) !== null,
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
    owner: cell && editsLikeOutline(cell) ? cellOwner(cell) : null,
    closeCompletion: completion.close, refreshCompletion: completion.refresh });
  const submitCapture = () => {
    if (!capture || !view) return false;
    commit(view.state.doc.toString());
    capture.submit();
    return true;
  };
  const commands = cm6CommandHandlers({
    text: {
      text: () => view!.state.doc.toString(), start: () => view!.state.selection.main.from,
      apply: edit => {
        completion.close();
        view!.dispatch({ changes: { from: 0, to: view!.state.doc.length, insert: edit.text },
          selection: { anchor: edit.start, head: edit.end }, scrollIntoView: true, userEvent: "input" });
      },
      move: pos => view!.dispatch({ selection: { anchor: pos }, scrollIntoView: true }),
    },
    view: () => view!, structure: structure.commands, formatting: formatting.commands,
    completion: completion.commands, clipboard: clipboard.commands,
    capture: submitCapture, upload: () => { void assets.uploadAsset(); },
    photo: () => { void assets.capturePhotoCmd(); }, voice: () => { void assets.voiceMemoToggle(); },
  });
  const command = (id: string) => runGridOutline(cell, () => sheet?.blocks(id) || commands[id]?.() || false);
  const registerCommandBridge = () => {
    removeCommandBridge?.();
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
      textCaret: () => view?.hasFocus && current()
        ? { text: view.state.doc.toString(), caret: view.state.selection.main.head } : null,
      dispatch: id => {
        typing.beforeKey();
        if (!view || !current() || typing.active()) return false;
        return command(id);
      },
    });
  };
  const exitEditOnBlur = (v: EditorView, handoff?: HTMLElement, intentIsCurrent = captureEditIntent()) => {
    commitBlockEditorExit({ id: props.id, text: v.state.doc.toString(), format, literal: literal(), handoff: !!handoff, commit });
    const endIfCurrent = () => {
      if (!intentIsCurrent() || editingId() !== props.id) return;
      if (handoff?.isConnected && handoff.ownerDocument.activeElement === handoff) return;
      if (handoff) finishPageHeaderEdit(props.id);
      endEdit("blur");
    };
    if (handoff) queueMicrotask(endIfCurrent);
    else endIfCurrent();
  };
  const windowFocus = editorWindowFocus({ id: props.id, element: () => view?.contentDOM,
    selection: () => ({ anchor: view!.state.selection.main.anchor, head: view!.state.selection.main.head }),
    focus: saved => {
      if (!view || !current()) return;
      if (saved) view.dispatch({ selection: { anchor: Math.min(saved.anchor, view.state.doc.length), head: Math.min(saved.head, view.state.doc.length) } });
      view.focus();
    },
    exit: () => { if (view && current()) exitEditOnBlur(view); },
  });
  // The view wraps a heading block's header line in `<span class="heading-text hN">` inside the block's own
  // line box; the editor wraps its first line in the same span, so both resolve the line box the same way
  // and entering or leaving edit does not move anything (CM6-HEADING). Level 1..6 from the block's own facets
  // (the `heading::` property is not in the editor text), the depth for `heading:: true`.
  const headingLevel = createMemo(() => effectiveHeadingLevel(propertySession.facets(source(), format), depthOf(props.id)));
  const buildDecorations = (state: EditorState): DecorationSet => {
    if (!parserReady() || literal()) return Decoration.none;
    const raw = state.doc.toString();
    const ranges = concealedSpans(previewSpans(raw, format), state.selection.ranges).map(span =>
      Decoration.replace({ widget: new PreviewWidget(span, raw, format, owner, props.id), block: !!span.block }).range(span.from, span.to));
    const level = headingLevel();
    const headerEnd = state.doc.line(1).to;
    if (level !== null && headerEnd > 0) ranges.push(Decoration.mark({ class: `heading-text h${level}` }).range(0, headerEnd));
    return Decoration.set(ranges, true);
  };
  const decorations = StateField.define<DecorationSet>({
    create: buildDecorations,
    update: (value, tr) => tr.docChanged || tr.selection || tr.effects.some(effect => effect.is(refreshPreview)) ? buildDecorations(tr.state) : value,
    provide: field => EditorView.decorations.from(field),
  });
  onMount(() => {
    view = new EditorView({
      parent: editorHost,
      dispatchTransactions: (transactions, v) => {
        updating = true;
        try { v.update(transactions); } finally { updating = false; }
        syncView();
        if (transactions.some(tr => tr.isUserEvent("select"))) enterBody();
      },
      state: EditorState.create({ doc: shown(), extensions: [
        cm6CodeHighlight(state => {
          if (isCalc()) return [];
          const projected = code();
          if (projected) return [{ from: 0, to: state.doc.length, lang: projected.lang }];
          return codeFences(state.doc.toString(), format).filter(f => f.lang !== "calc" && f.openEnd < f.closeStart)
            .map(f => ({ from: f.openEnd, to: Math.min(f.closeStart, state.doc.length), lang: f.kind === "example" ? "" : f.lang }));
        }),
        decorations, wrapping.of(!isCalc() && (!codeEditing() || codeWrapping()) ? EditorView.lineWrapping : []),
        editability.of([EditorState.readOnly.of(!current()), EditorView.editable.of(current())]), completion.inputSelection, typing.filter,
        spellcheck.of(EditorView.contentAttributes.of({ spellcheck: String(spellcheckEnabled()) })),
        EditorView.editorAttributes.of({ "data-block-editor": "cm6" }),
        EditorView.contentAttributes.of({ "data-cm6-editor": props.id, "aria-label": "" }),
        keymap.of(defaultKeymap), cm6EmptyCaret(),
        EditorView.domEventHandlers({
          focus: () => { noteSurfaceFocused(props.surface); registerCommandBridge(); return false; },
          blur: (event, v) => {
            clipboard.dispose();
            const abandoned = typing.active() && !typing.settling(); // preedit still open at blur (L9-18)
            const intentIsCurrent = captureEditIntent();
            // Reorders can blur a retained editor; the structural gesture owns
            // its focus restoration. Palette actions still need embed context.
            const leave = () => {
              if (!current() || !intentIsCurrent() || editingId() !== props.id) return;
              if (assets.holdsBlur() || isBlockMoving() || (event.relatedTarget as HTMLElement | null)?.closest?.(".date-picker")
                || inPageFindPreservesEditorBlur()
                || embedExit && (event.relatedTarget as HTMLElement | null)?.closest?.(".switcher")
                || !documentHasFocus(v.contentDOM)) {
                commit(v.state.doc.toString());
                windowFocus.remember();
                return;
              }
              if (v.hasFocus) { commit(v.state.doc.toString()); return; }
              const target = event.relatedTarget as HTMLElement | null;
              const handoff = target?.matches('[data-cm6-editor]')
                && target.getAttribute("data-cm6-editor") === props.id ? target : null;
              exitEditOnBlur(v, handoff ?? undefined, intentIsCurrent);
            };
            // compositionend's pending DOM reconciliation owns its last flush.
            // A blur during preedit still commits the live buffer before exit.
            if (typing.settling()) queueMicrotask(leave);
            else leave();
            // An IME cancelled by blur may send no compositionend, and CM never clears its own composing flag
            // without one. End the preedit as the IME would, but only after leave() has committed the live buffer
            // (ending it first defers leave() and loses the draft), so the next ordinary edit is not swallowed.
            if (abandoned) v.contentDOM.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
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
    formatting.mount();
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
      // Tab and native reload chords are pre-cancelled there while their bound
      // editor commands still own dispatch. Transients stop propagation upstream.
      if (event.defaultPrevented && !isPermittedTabGesture(event)
        && !(isWebviewReloadChord(event) && editorCommandFor(event))) return;
      // Run before CM's target handler: desktop WebKit suppresses its keymaps
      // just after compositionend, including an already committed plain Enter.
      if (completion.keydown(event)) {
        event.preventDefault(); event.stopImmediatePropagation(); return;
      }
      if (commentKey(event, props.id, () => view!.dispatch(view!.state.replaceSelection("\n"), { userEvent: "input" }), () => { commit(view!.state.doc.toString()); endEdit("page-navigation"); })) {
        event.stopImmediatePropagation(); return;
      }
      const id = editorCommandFor(event);
      const intent = captureKeyIntent(event, id, capture);
      if (intent) {
        if (intent === "submit") submitCapture();
        else capture!.cancel();
        event.preventDefault(); event.stopImmediatePropagation(); return;
      }
      if (event.key === "Escape" && !event.ctrlKey && !event.metaKey && !event.altKey) {
        const raw = codeView.revealRaw(view.state.doc.toString(), commit);
        if (raw !== null) {
          showText(view, raw, raw.length);
          view.focus();
          event.preventDefault(); event.stopImmediatePropagation(); return;
        }
      }
      const s = view.state.selection.main;
      const leave = codeView.leaveRange(event, view.state.doc.toString(), s.from, s.to);
      if (leave) showText(view, visible(), leave.anchor, leave.head);
      let handled: boolean;
      try {
        handled = !!runGridOutline(cell, () => leave || clipboard.keydown(event, view!) || sheet?.keydown(event, view!) || id && command(id) || formatting.keydown(event) || structure.keydown(event, view!));
      } catch (error) {
        // A command that threw (a refused commit, a parser trap before the kernel answered) still
        // owns its key: CodeMirror's own binding would put the Enter or Tab into the text, and the
        // next keystroke would act on that instead (og-wasm-enter-trap). The error stays uncaught.
        event.preventDefault();
        event.stopImmediatePropagation();
        throw error;
      }
      if (handled) {
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
    /** `again`: a later request to this mounted view (see focusMountedEditor), applied, not taken. */
    const focusNow = (again?: { offset: CaretPos }) => {
      if (!view) return;
      // Mirrored occurrences share this request. Only the view taking focus
      // may consume it, including when a stale surface stamp needs a fallback.
      // After its block took a calculator's text (a template in the empty block) the view enters calc
      // mode, as a fresh mount would.
      if (!isCalc() && calcSource(visible()) !== null) setCalc(true);
      const want = again ? again.offset : takeCaretFor(props.id);
      const historySelection = again ? null : takeHistoryEditorSelectionFor(props.id, props.surface);
      // Numeric targets use visible-source coordinates. History uses body
      // coordinates for whole code and source coordinates for mixed occurrences.
      if (!code() && !isCalc()) {
        if (historySelection) codeView.enter(historySelection.start, historySelection.end);
        else if (typeof want === "number") codeView.enter(want);
        else if (want && "start" in want) codeView.enter(want.start, want.end);
        syncView();
      }
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
      view.focus();
    };
    focusMountedEditor(props.id, props.surface, () => view!.contentDOM, again => {
      if (current()) focusNow(again);
    });
    const caretReveal = cm6CaretReveal(view);
    view.dispatch({ effects: StateEffect.appendConfig.of(caretReveal.extension) });
    onCleanup(caretReveal.cancel);
    if (view.hasFocus) registerCommandBridge();
  });
  createEffect(() => {
    if (!current()) return;
    shown(); codeEditing();
    syncView();
  });
  // Raw text invalidates these policies, but usually leaves their values
  // unchanged. Reconfigure only when the policy changes, and keep reads made
  // by CM's synchronous listeners outside the reactive subscription.
  const wrapPolicy = createMemo(() => !isCalc() && (!codeEditing() || codeWrapping()));
  const writablePolicy = createMemo(current);
  createEffect(() => {
    const wrap = wrapPolicy();
    untrack(() => view?.dispatch({ effects: wrapping.reconfigure(wrap ? EditorView.lineWrapping : []) }));
  });
  createEffect(() => {
    const writable = writablePolicy();
    untrack(() => view?.dispatch({ effects: editability.reconfigure([EditorState.readOnly.of(!writable), EditorView.editable.of(writable)]) }));
  });
  createEffect(() => {
    const enabled = spellcheckEnabled();
    untrack(() => view?.dispatch({ effects: spellcheck.reconfigure(EditorView.contentAttributes.of({ spellcheck: String(enabled) })) }));
  });
  createEffect(() => {
    if (parserReady()) untrack(() => view?.dispatch({ effects: refreshPreview.of(null) }));
  });
  const highlightContext = createMemo(() => JSON.stringify([parserReady(), isCalc(), code()?.lang]));
  createEffect(() => {
    highlightContext();
    untrack(() => view?.dispatch({ effects: refreshCodeHighlight.of(null) }));
  });
  createEffect(() => {
    headingLevel();
    untrack(() => view?.dispatch({ effects: refreshPreview.of(null) }));
  });
  onCleanup(() => {
    pendingScrollAnchor?.cancel();
    if (anchorFrame !== undefined) host.ownerDocument.defaultView!.cancelAnimationFrame(anchorFrame);
    alive = false; typing.dispose(); clipboard.dispose(); removeCommandBridge?.(); removeKeys?.(); view?.destroy();
  });
  return <div class="cm6-block-editor editor-wrap" dir={blockDirAttr(source(), format)} classList={{ "calc-wrap": isCalc(), "code-wrapping": codeEditing() && codeWrapping() }} ref={host}>
    <Show when={mixedContext()?.before}>{before => <div class="block-content mixed-code-before" onMouseDown={event => editMixedContext(event, "before")}><AstBody raw={before()} format={format} headingLevel={effectiveHeadingLevel(propertySession.facets(source(), format), depthOf(props.id))} /></div>}</Show>
    <div class="code-editor-region" style={codeGutterStyle(codeLines().length)} dir={codeEditing() || isCalc() ? "ltr" : undefined} classList={{ "mixed-code-region": !!mixedContext() }}>
      <Show when={codeEditing()}>
        <span class="code-language">{(code() ?? codeFenceOnly(visible(), format))?.lang.toLowerCase()}</span>
        <Show when={code() ?? codeBodyProjection(visible(), format)}>
          {body => <CopyButton text={body().body} title={translate("render-copy-code")} class="code-copy" preserveFocus />}
        </Show>
        <LineGutter lines={codeLines()} code />
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

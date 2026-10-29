import { EditorSelection } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import {
  beginPageHeaderEdit, blockHasFoldableBody, deleteBlock, indentBlock, insertOutlineAfter,
  mergeWithNext, mergeWithPrev, moveBlockFeed, moveItem, moveSelection, nextVisible,
  node, outdentBlock, pageByName, prevVisible, selectBlock,
  selectBlockSubtree, setCollapsed, splitBlock, withBlockMoving, withUndoUnit, type OutlineScope,
  removeOwnNumberedList, stopOwnNumberedListOnEmptyEnter,
  setRaw,
} from "../document";
import { captureEditIntent, editingId, startEditing, type EditorSelection as CaretSelection } from "../editorController";
import { navigateJournalDown } from "./journalNavigation";
import { isPermittedTabGesture, isTabLikeEvent } from "../keybindings";
import { guardedCommands } from "./cm6Commands";
import { noticeMoveOutOfView } from "../moveNotice";
import { pushToast } from "../toasts";
import { msg } from "../i18n";
import { autoPairing, documentMode, docModeEnterForNewBlock } from "../ui";
import { backspacePairEdit } from "./autopair";
import { replyOnEnter } from "../commentActions";
import { detectMacro } from "../components/Rendered";
import { listLineAt } from "../components/blockParts";
import { type CollapseSurfaceApi } from "../components/Block";
import { isBlockEmbedMacro } from "../components/blockParts";
import { isAnnotationBlock } from "./annotation";
import { calcSource } from "./calc";
import { caretInFence, caretInDisplayMath, caretOnOpeningFence } from "./fences";
import { codeBodyExitTrim } from "./codeFence";
import { BUILTIN_HIDE, caretOnPropertyLine, isEditorEmpty, isPropertiesOnly, multilineExitTrim } from "./properties";

interface StructureContext {
  id: string;
  surface: string;
  scope: OutlineScope | null;
  format: "md" | "org";
  current(): boolean;
  commit(text: string): void;
  code(): boolean;
  calc(): boolean;
  annotation(): boolean;
  viewPrev(): string | null;
  viewNext(): string | null;
  embedExit: { hostBlockId: string; firstRoot(): string | undefined } | null;
  collapse: CollapseSurfaceApi | null;
}

/** CM owns text/visual rows; document commands own outline topology and writes.
 * The boundary receives the same occurrence and navigation context as Editor. */
export function cm6Structure(ctx: StructureContext) {
  const editSurface = () => ctx.surface.startsWith("embed:") ? ctx.surface : null;
  const navSurface = () => ctx.surface.startsWith("ref:") || ctx.surface.startsWith("embed:") ? ctx.surface : null;
  const structuralScope = ctx.scope?.navOnly ? null : ctx.scope;
  const selection = (v: EditorView): CaretSelection => {
    const s = v.state.selection.main;
    return { start: s.from, end: s.to, direction: s.empty ? "none" : s.anchor > s.head ? "backward" : "forward" };
  };
  const softNewline = (v: EditorView) => {
    v.dispatch(v.state.replaceSelection("\n"), { scrollIntoView: true, userEvent: "input" });
    return true;
  };
  const atEdge = (v: EditorView, forward: boolean) => {
    const s = v.state.selection.main;
    return v.moveToLineBoundary(EditorSelection.cursor(s.head), forward).head === (forward ? v.state.doc.length : 0);
  };
  const restore = (v: EditorView, s: CaretSelection) => {
    if (!ctx.current() || !v.dom.isConnected || editingId() !== ctx.id) return;
    const end = Math.min(s.end, v.state.doc.length), start = Math.min(s.start, end);
    v.dispatch({ selection: s.direction === "backward" ? EditorSelection.single(end, start) : EditorSelection.single(start, end) });
    v.focus();
  };
  const nudge = (v: EditorView, out: boolean) => {
    const s = selection(v), text = v.state.doc.toString();
    const ll = ctx.code() || ctx.calc() ? null : listLineAt(text, s.start, ctx.format);
    if (ll && (ctx.format === "org" || ll.lineStart > 0) && (!out || ll.indent.length > 0)) {
      const removed = out ? Math.min(2, ll.indent.length) : 0;
      const caret = out ? Math.max(ll.lineStart, s.start - removed) : s.start + 2;
      v.dispatch({ changes: { from: ll.lineStart, to: ll.lineStart + removed, insert: out ? "" : "  " }, selection: { anchor: caret }, userEvent: "input" });
      return true;
    }
    if (out ? ctx.scope?.forceExpandedRoot === node(ctx.id)?.parent : !ctx.scope?.navOnly && ctx.scope?.roots.includes(ctx.id)) return true;
    ctx.commit(text);
    if (out) outdentBlock(ctx.id, s, editSurface());
    else if (indentBlock(ctx.id, s, editSurface()) === false) pushToast(msg("editor-block-outline-is-too-deep-to-indent"), "error");
    return true;
  };
  const move = (v: EditorView, dir: -1 | 1) => {
    const s = selection(v);
    ctx.commit(v.state.doc.toString());
    void withBlockMoving(node(ctx.id).page, async () => {
      startEditing(ctx.id, s, null, editSurface());
      const intentIsCurrent = captureEditIntent();
      const moving = ctx.scope && !ctx.scope.navOnly ? moveItem(ctx.id, dir) : moveBlockFeed(ctx.id, dir);
      const active = () => v.dom.ownerDocument.activeElement;
      if (intentIsCurrent() && (active() === v.contentDOM || active() === v.dom.ownerDocument.body)) restore(v, s);
      const moved = await moving;
      if (ctx.current() && moved !== false && moved !== "none") noticeMoveOutOfView(ctx.id, ctx.scope);
      await new Promise<void>(resolve => v.dom.ownerDocument.defaultView!.requestAnimationFrame(() => resolve()));
      // A DOM reorder can clear the browser range while retaining focus. CM's
      // observer reconciles that range on its frame; restore both state and DOM.
      if (intentIsCurrent()) restore(v, s);
    }).catch(() => console.error("Block move failed"));
    return true;
  };
  const collapse = (value: boolean) => {
    if (!ctx.collapse) setCollapsed(ctx.id, value);
    else if (blockHasFoldableBody(ctx.id)) {
      const current = ctx.collapse.collapsed(ctx.id, node(ctx.id).collapsed);
      if (current !== value) ctx.collapse.toggle(ctx.id, current);
    }
    return true;
  };
  const selectBlockAtEdge = (v: EditorView, dir: -1 | 1) => {
    if (!atEdge(v, dir > 0)) return false;
    selectBlock(ctx.id, ctx.scope);
    moveSelection(dir, true);
    return true;
  };
  const commands = guardedCommands<[EditorView]>(v => ctx.current() && !v.compositionStarted, {
    "editor/indent": v => nudge(v, false),
    "editor/outdent": v => nudge(v, true),
    "editor/move-block-up": v => move(v, -1),
    "editor/move-block-down": v => move(v, 1),
    "editor/collapse": () => collapse(true),
    "editor/expand": () => collapse(false),
    "editor/soft-newline": softNewline,
    "editor/select-embedding-block": v => {
      if (!ctx.embedExit) return false;
      const host = v.dom.closest(".embed-block, .embed-page")?.closest<HTMLElement>(".ls-block");
      selectBlock(ctx.embedExit.hostBlockId);
      host?.querySelector<HTMLElement>(":scope > .block-main")?.scrollIntoView?.({ block: "nearest" });
      return true;
    },
    "editor/select-all": v => {
      const s = v.state.selection.main;
      if (s.from !== 0 || s.to !== v.state.doc.length) return false;
      selectBlockSubtree(ctx.id, ctx.scope);
      return true;
    },
    "editor/select-block-up": v => selectBlockAtEdge(v, -1),
    "editor/select-block-down": v => selectBlockAtEdge(v, 1),
  });
  const enter = (v: EditorView) => {
    const s = v.state.selection.main, raw = v.state.doc.toString();
    const n = node(ctx.id);
    const macro = ctx.embedExit ? detectMacro(node(ctx.embedExit.hostBlockId)?.raw ?? "", ctx.format) : null;
    if (ctx.embedExit && ctx.id === ctx.embedExit.firstRoot() && isBlockEmbedMacro(macro)
      && ctx.collapse?.collapsed(ctx.id, n.collapsed)
      && s.empty && s.to === raw.length) {
      const id = insertOutlineAfter(ctx.embedExit.hostBlockId, [{ raw: "", children: [] }]);
      if (id) startEditing(id, 0);
      return true;
    }
    // Highlight text and hidden metadata remain one block, even when Enter
    // starts from a selection. Annotation ownership comes from the property
    // session, before interpreting any projected text as code or properties.
    if (ctx.annotation()) {
      ctx.commit(raw);
      const id = insertOutlineAfter(ctx.id, [{ raw: "", children: [] }]);
      if (id) startEditing(id, 0, null, editSurface());
      return true;
    }
    const page = pageByName(n.page);
    const properties = ctx.format === "md" && page?.roots[0] === ctx.id
      && (n.originatedFromPageHeader || (!page.preBlock && (isPropertiesOnly(raw) || (raw.endsWith("\n") && isPropertiesOnly(raw.slice(0, -1))))));
    const fence = !ctx.code() && !ctx.calc() && caretInFence(raw, s.from, ctx.format);
    const math = !ctx.code() && !ctx.calc() && !fence && caretInDisplayMath(raw, s.from, ctx.format);
    const kind = ctx.calc() ? "calc" : ctx.code() ? "code" : fence ? "fence" : math ? "math" : properties ? "properties" : null;
    if (kind && s.empty) {
      const trimmed = kind === "code" ? codeBodyExitTrim(raw, s.from) : multilineExitTrim(raw, s.from, kind, ctx.format);
      if (trimmed !== null) {
        let id: string | null = null;
        withUndoUnit(`multiline-exit:${ctx.id}`, [n.page], () => {
          ctx.commit(trimmed);
          id = insertOutlineAfter(ctx.id, [{ raw: "", children: [] }]);
        });
        if (id) startEditing(id, 0, null, editSurface());
        return true;
      }
    }
    if (properties && caretOnPropertyLine(raw, s.from)) return softNewline(v);
    if (ctx.code() || ctx.calc() || fence || math || caretOnOpeningFence(raw, s.from, ctx.format)) return softNewline(v);
    if (stopOwnNumberedListOnEmptyEnter(ctx.id, raw)) {
      v.dispatch({ selection: { anchor: 0 } });
      return true;
    }
    const ll = calcSource(raw) === null ? listLineAt(raw, s.from, ctx.format) : null;
    if (ll) {
      const nextMarker = /\d/.test(ll.marker) ? parseInt(ll.marker) + 1 + ll.marker.replace(/\d+/, "") : ll.marker;
      const insert = "\n" + ll.indent + nextMarker + " " + (ll.hasCheckbox ? "[ ] " : "");
      v.dispatch({ changes: { from: s.from, insert }, selection: { anchor: s.from + insert.length }, userEvent: "input" });
      return true;
    }
    if (!s.empty) v.dispatch({ changes: { from: s.from, to: s.to } });
    ctx.commit(v.state.doc.toString());
    if (s.empty && replyOnEnter(ctx.id, raw, s.from, editSurface())) return true;
    const zoom = ctx.scope?.forceExpandedRoot === ctx.id;
    splitBlock(ctx.id, s.from, zoom, zoom, editSurface());
    return true;
  };
  const vertical = (v: EditorView, forward: boolean) => {
    if (!v.state.selection.main.empty || !atEdge(v, forward)) return false;
    const s = v.state.selection.main;
    const col = s.head - v.moveToLineBoundary(s, false).head;
    let target = forward ? ctx.viewNext() : ctx.viewPrev();
    const exiting = !forward && ctx.embedExit !== null && (!target || ctx.embedExit.firstRoot() === ctx.id);
    if (exiting) target = prevVisible(ctx.embedExit!.hostBlockId);
    if (!forward && !target && !ctx.scope && (ctx.surface === "main" || ctx.surface.startsWith("pane:"))
      && pageByName(node(ctx.id).page)?.roots[0] === ctx.id) target = beginPageHeaderEdit(node(ctx.id).page);
    if (target) startEditing(target, { col, edge: forward ? "first" : "last" }, null, exiting ? null : navSurface());
    else if (forward && !ctx.scope) {
      void navigateJournalDown({ id: ctx.id, col, surface: navSurface(), editor: v.contentDOM,
        current: editor => ctx.current() && editor === v.contentDOM });
    } else return false;
    return true;
  };
  const keydown = (e: KeyboardEvent, v: EditorView) => {
    if (!ctx.current() || v.compositionStarted || e.isComposing || e.keyCode === 229) return false;
    if (isTabLikeEvent(e) && !isPermittedTabGesture(e)) return false;
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    if (e.key === "Enter" && plain) {
      const newline = documentMode() && !docModeEnterForNewBlock();
      return e.shiftKey !== newline ? softNewline(v) : enter(v);
    }
    const s = v.state.selection.main, raw = v.state.doc.toString();
    if (plain && s.empty && ctx.annotation()
      && (e.key === "Backspace" && s.head === 0 || e.key === "Delete" && s.head === raw.length)) return false;
    if (plain && e.key === "Escape") { selectBlock(ctx.id, ctx.scope); return true; }
    if (plain && s.empty && e.key === "Backspace") {
      if (ctx.code() && raw === "" && !node(ctx.id).children.length) {
        const n = node(ctx.id);
        const adjacent = [prevVisible(ctx.id, structuralScope), nextVisible(ctx.id, structuralScope)].find(id => id && node(id)?.page === n.page);
        if (adjacent) { deleteBlock(ctx.id); startEditing(adjacent, 0, null, editSurface()); }
        else setRaw(ctx.id, "", { timetracking: false });
        return true;
      }
      const pair = backspacePairEdit(raw, s.head);
      if (pair && (autoPairing() || raw[s.head - 1] === "[" && raw[s.head] === "]")) {
        v.dispatch({ changes: { from: 0, to: raw.length, insert: pair.text }, selection: { anchor: pair.start }, userEvent: "delete.backward" });
        return true;
      }
      const ll = !ctx.code() && !ctx.calc() && calcSource(raw) === null ? listLineAt(raw, s.head, ctx.format) : null;
      if (ll && s.head === ll.lineStart + ll.prefixLen) {
        v.dispatch({ changes: { from: ll.lineStart, to: s.head }, selection: { anchor: ll.lineStart }, userEvent: "delete.backward" });
        return true;
      }
    }
    if (plain && s.empty && e.key === "Backspace" && s.head === 0) {
      if (ctx.code() || ctx.calc()) return true;
      if (calcSource(raw) !== null) return false;
      if (removeOwnNumberedList(ctx.id)) {
        v.dispatch({ selection: { anchor: 0 } });
        return true;
      }
      if (mergeWithPrev(ctx.id, structuralScope, editSurface())) return true;
      const n = node(ctx.id), next = nextVisible(ctx.id, structuralScope);
      if (isEditorEmpty(n.raw, BUILTIN_HIDE, ctx.format) && !n.children.length && next && node(next)?.page === n.page) {
        deleteBlock(ctx.id); startEditing(next, 0, null, editSurface()); return true;
      }
    }
    if (plain && s.empty && e.key === "Delete" && s.head === raw.length) {
      if (ctx.code() || ctx.calc()) return true;
      if (calcSource(raw) !== null) return false;
      const next = nextVisible(ctx.id, structuralScope), nextRaw = next ? node(next)?.raw ?? "" : "";
      if (next && !isAnnotationBlock(nextRaw, ctx.format) && calcSource(nextRaw) === null && mergeWithNext(ctx.id, structuralScope, editSurface())) {
        if (ctx.current()) v.dispatch({ selection: { anchor: s.head } });
        return true;
      }
    }
    if (plain && !e.shiftKey && s.empty) {
      if (e.key === "ArrowLeft" && s.head === 0 || e.key === "ArrowRight" && s.head === raw.length) {
        const forward = e.key === "ArrowRight", target = forward ? ctx.viewNext() : ctx.viewPrev();
        if (target) { startEditing(target, forward ? 0 : Number.MAX_SAFE_INTEGER, null, navSurface()); return true; }
      }
      if (e.key === "ArrowUp" || e.key === "ArrowDown") return vertical(v, e.key === "ArrowDown");
    }
    return false;
  };
  return { commands, keydown };
}

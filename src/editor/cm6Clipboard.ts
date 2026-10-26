import type { EditorView } from "@codemirror/view";
import { clearClipboardSlot, normalize, peekClipboardSlot, readClipboardText } from "../clipboard";
import { insertOutlineAfter, insertOutlineChildren, node, outlineFits, pasteClipboardPayload, replaceEmptyBlockWithOutline, sanitizeOutlineIdsForPaste, withUndoUnit } from "../document";
import { startEditing } from "../editorController";
import { pasteAsLinkEdit } from "../blockRefActions";
import { copyBlockLink } from "../components/blockLinkCopy";
import { editorCommandFor } from "../keybindings";
import { shortcutKey } from "../shortcutKey";
import { graphOwner, readOwned } from "../owned";
import { reportUiFailure } from "../uiFailure";
import { msg } from "../i18n";
import { pushToast } from "../toasts";
import { structuralSheetPasteNode } from "../sheet/mutations";
import { caretInDisplayMath, caretInFence, caretOnOpeningFence } from "./fences";
import { codeFenceOnly } from "./codeFence";
import { calcSource } from "./calc";
import { isEditorEmpty, type HidePolicy } from "./properties";
import { OUTLINE_MAX_SOURCE_CHARS, pastedPlainBlocks, type OutlineNode } from "./outline";
import { structuredHtmlPaste } from "./htmlPaste";
import { isPasteableUrl, videoPasteMacro, wrapLink } from "./format";

interface ClipboardContext {
  id: string;
  format: "md" | "org";
  sheet: boolean;
  hiddenProperties: HidePolicy;
  current(): boolean;
  commit(text: string): void;
  literal(): boolean;
}

/** Adapt the existing paste policies to CM state. Outline and identity changes
 * still go through the document commands; text changes use CM transactions. */
export function cm6Clipboard(ctx: ClipboardContext) {
  let rawPaste = false;
  let rawTimer: ReturnType<typeof setTimeout> | undefined;
  const clearRaw = () => {
    rawPaste = false;
    if (rawTimer !== undefined) clearTimeout(rawTimer);
    rawTimer = undefined;
  };
  const available = (v: EditorView) => ctx.current() && v.dom.isConnected && !v.compositionStarted;
  const apply = (v: EditorView, edit: { text: string; start: number; end: number }) => {
    v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: edit.text },
      selection: { anchor: edit.start, head: edit.end }, userEvent: "input.paste", scrollIntoView: true });
  };
  const literal = (v: EditorView, text: string) => {
    v.dispatch(v.state.replaceSelection(text), { userEvent: "input.paste", scrollIntoView: true });
  };
  const insertOutline = (v: EditorView, nodes: OutlineNode[], tag: string, asChildren = false) => {
    const host = node(ctx.id);
    if (!host || !available(v)) return;
    if (!outlineFits(ctx.id, nodes, asChildren ? 1 : 0)) {
      pushToast(msg("editor-block-pasted-outline-is-too-deep"), "error"); return;
    }
    const rawAtPaste = v.state.doc.toString(), pageAtPaste = host.page;
    const owner = graphOwner(() => available(v) && v.state.doc.toString() === rawAtPaste
      && node(ctx.id)?.page === pageAtPaste);
    const insert = (prepared: OutlineNode[] | null) => {
      if (!prepared?.length || !owner()) return;
      if (asChildren) {
        const inserted = withUndoUnit(tag, [pageAtPaste], () => {
          ctx.commit(rawAtPaste);
          return insertOutlineChildren(ctx.id, prepared) ?? false;
        });
        if (!inserted) pushToast(msg("editor-block-pasted-outline-was-refused"), "error");
        return;
      }
      const lastId = withUndoUnit(tag, [pageAtPaste], () => {
        ctx.commit(rawAtPaste);
        const host = node(ctx.id);
        const empty = rawAtPaste.trim() === "" && isEditorEmpty(host.raw, ctx.hiddenProperties, ctx.format) && host.children.length === 0;
        return (empty ? replaceEmptyBlockWithOutline(ctx.id, prepared) : insertOutlineAfter(ctx.id, prepared)) ?? false;
      });
      if (!lastId) pushToast(msg("editor-block-pasted-outline-was-refused"), "error");
      else if (node(lastId)) startEditing(lastId, node(lastId).raw.length);
    };
    const prepared = sanitizeOutlineIdsForPaste(ctx.id, nodes);
    if (prepared instanceof Promise) void prepared.then(insert).catch(error => reportUiFailure("clipboard-association", error));
    else insert(prepared);
  };
  const pasteAsLink = async (v: EditorView) => {
    const before = v.state.doc.toString(), s = v.state.selection.main;
    const owner = graphOwner(() => available(v) && v.state.doc.toString() === before);
    let clipboard: string;
    try {
      const result = await readOwned(owner, readClipboardText());
      if (result.kind === "stale") return;
      clipboard = result.value;
    } catch {
      if (owner()) pushToast(msg("editor-block-couldn-t-read-the-clipboard"), "error");
      return;
    }
    const edit = pasteAsLinkEdit(before, s.from, s.to, clipboard, ctx.format);
    if (!edit) { pushToast(msg("editor-block-the-clipboard-doesn-t-hold-a-link-or-a-block-ref"), "warn"); return; }
    apply(v, edit);
  };
  const command = (v: EditorView, id: string) => {
    if (!available(v)) return false;
    if (id === "editor/paste-as-link") { void pasteAsLink(v); return true; }
    if (id === "editor/copy-embed" && v.state.selection.main.empty) {
      ctx.commit(v.state.doc.toString()); void copyBlockLink(ctx.id, "embed"); return true;
    }
    return false;
  };
  const keydown = (e: KeyboardEvent, v: EditorView) => {
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && shortcutKey(e) === "v") {
      clearRaw(); rawPaste = true; rawTimer = setTimeout(clearRaw, 1_000); return false;
    }
    clearRaw();
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && shortcutKey(e) === "c" && v.state.selection.main.empty) {
      ctx.commit(v.state.doc.toString()); void copyBlockLink(ctx.id, "ref"); return true;
    }
    const id = editorCommandFor(e);
    return id ? command(v, id) : false;
  };
  const paste = (e: ClipboardEvent, v: EditorView) => {
    const raw = rawPaste;
    clearRaw();
    if (!available(v)) return true;
    const text = e.clipboardData?.getData("text/plain") ?? "";
    if (raw) { if (text) literal(v, text); return true; }
    // File payloads belong to the later native-assets port. Do not interpret
    // their companion path text as an outline or a private block payload.
    const types = Array.from(e.clipboardData?.types ?? []);
    if (Array.from(e.clipboardData?.items ?? []).some(item => item.kind === "file")
      || types.some(type => type === "Files" || type === "text/uri-list" || type === "x-special/gnome-copied-files")) return false;
    const source = v.state.doc.toString(), s = v.state.selection.main;
    const code = codeFenceOnly(source, ctx.format) !== null;
    const syntaxSensitive = ctx.literal() || ctx.sheet || calcSource(source) !== null || code
      || caretInFence(source, s.from, ctx.format) || caretOnOpeningFence(source, s.from, ctx.format)
      || caretInDisplayMath(source, s.from, ctx.format);
    const slot = peekClipboardSlot();
    if (!syntaxSensitive && slot && text !== "" && normalize(text) === normalize(slot.text)) {
      void readOwned(graphOwner(() => ctx.current()), pasteClipboardPayload(ctx.id, slot)).then(result => {
        if (result.kind === "stale") return;
        const lastId = result.value;
        if (lastId && node(lastId)) startEditing(lastId, node(lastId).raw.length);
      }).catch(error => reportUiFailure("clipboard-association", error));
      return true;
    }
    if (slot && text !== "" && normalize(text) !== normalize(slot.text)) clearClipboardSlot();
    const grid = structuralSheetPasteNode(text);
    if (grid) { insertOutline(v, [grid], "sheet-outline-paste", true); return true; }
    const html = e.clipboardData?.getData("text/html") ?? "";
    const structured = syntaxSensitive ? null : structuredHtmlPaste(html, text, ctx.format);
    if (structured) {
      if (structured.inlineText !== null) literal(v, structured.inlineText);
      else insertOutline(v, structured.outline, "structured-paste");
      return true;
    }
    if (text.includes("\n")) {
      if (text.length > OUTLINE_MAX_SOURCE_CHARS) { pushToast(msg("editor-block-pasted-text-is-too-large"), "error"); return true; }
      const nodes = syntaxSensitive ? null : pastedPlainBlocks(text, ctx.format);
      if (nodes === null) literal(v, text);
      else if (nodes.length) insertOutline(v, nodes, "outline-paste");
      return true;
    }
    const url = text.trim();
    if (!s.empty && isPasteableUrl(url) && !isPasteableUrl(source.slice(s.from, s.to))
      && !ctx.literal() && calcSource(source) === null && !code && !caretInFence(source, s.from, ctx.format) && !caretOnOpeningFence(source, s.from, ctx.format)) {
      apply(v, wrapLink(source, s.from, s.to, url, ctx.format)); return true;
    }
    const video = videoPasteMacro(text);
    if (video && s.empty) { literal(v, video); return true; }
    return false;
  };
  return { paste, keydown, command, clearRaw };
}

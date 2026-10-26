import { createEffect, createSignal, createUniqueId, For, onCleanup, Show, useContext } from "solid-js";
import type { EditorView, ViewUpdate } from "@codemirror/view";
import { essentialSelectionActions, secondarySelectionActions, type SelectionAction } from "./selectionActions";
import { insertLink, toggleInlineFormat, type Edit, type InlineFormat } from "./format";
import { doubleRefKind, SELECTION_WRAP, wrapSelectionEdit } from "./autopair";
import { cycleMarkerSmart } from "./repeat";
import { workflow } from "../ui";
import { commentFromEditor } from "../commentActions";
import { CaptureCtx } from "../components/Block";
import { selectionActionLabel } from "../components/SelectionActionLabel";
import { registerTransientLayer } from "../transientLayers";
import { touchPrimary } from "../inputCapability";
import { t } from "../i18n";
import { guardedCommands } from "./cm6Commands";

const inlineCommands: Readonly<Record<string, InlineFormat>> = {
  "editor/bold": "bold", "editor/italics": "italic",
  "editor/strike-through": "strikethrough", "editor/highlight": "highlight",
};

/** Formatting shares the textarea's pure edits and document commands. CM owns
 * the selection, including its active end; no DOM draft or second undo stack. */
export function cm6Formatting(ctx: {
  id: string; surface: string; format: "md" | "org";
  current(): boolean; view(): EditorView | undefined; commit(text: string): void;
  closeCompletion(): void; refreshCompletion(): void;
}) {
  const cap = useContext(CaptureCtx);
  const [selected, setSelected] = createSignal(false);
  const [overflow, setOverflow] = createSignal(false);
  let overflowRef: HTMLDivElement | undefined;
  const layerId = `cm6-selection-overflow-${createUniqueId()}`;
  const current = () => ctx.current() && !!ctx.view()?.dom.isConnected && !ctx.view()!.compositionStarted;
  const apply = (edit: Edit) => {
    if (!current()) return;
    const view = ctx.view()!;
    ctx.closeCompletion();
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: edit.text },
      selection: edit.direction === "backward" ? { anchor: edit.end, head: edit.start } : { anchor: edit.start, head: edit.end },
      scrollIntoView: true, userEvent: "format" });
    view.focus();
    setOverflow(false);
  };
  const action = (action: SelectionAction) => {
    if (!current()) return;
    const view = ctx.view()!, s = view.state.selection.main;
    apply(action.apply(view.state.doc.toString(), s.from, s.to, ctx.format, s.anchor > s.head ? "backward" : "forward"));
  };
  const comment = () => {
    if (!current() || cap) return;
    const view = ctx.view()!, s = view.state.selection.main;
    ctx.closeCompletion();
    commentFromEditor(ctx.id, { value: view.state.doc.toString(), selectionStart: s.from, selectionEnd: s.to },
      ctx.commit, ctx.surface.startsWith("embed:") ? ctx.surface : null);
  };
  const commands = guardedCommands(current, {
    ...Object.fromEntries(Object.entries(inlineCommands).map(([id, kind]) => [id, () => {
      const view = ctx.view()!, s = view.state.selection.main;
      apply(toggleInlineFormat(view.state.doc.toString(), s.from, s.to, ctx.format, kind, s.anchor > s.head ? "backward" : "forward"));
      return true;
    }])),
    "editor/insert-link": () => {
      const view = ctx.view()!, s = view.state.selection.main;
      apply(insertLink(view.state.doc.toString(), s.from, s.to, ctx.format));
      return true;
    },
    "editor/comment": () => { comment(); return true; },
    "editor/cycle-todo": () => {
      const view = ctx.view()!, s = view.state.selection.main, text = view.state.doc.toString();
      const { raw, delta } = cycleMarkerSmart(text, workflow(), ctx.format);
      const pos = Math.max(0, Math.min(raw.length, s.from + delta));
      apply({ text: raw, start: pos, end: pos });
      return true;
    },
  });
  const keydown = (event: KeyboardEvent) => {
    if (!current()) return false;
    const view = ctx.view()!, s = view.state.selection.main;
    if (s.empty || event.ctrlKey || event.metaKey || event.isComposing || event.keyCode === 229
      || !Object.prototype.hasOwnProperty.call(SELECTION_WRAP, event.key)) return false;
    const edit = wrapSelectionEdit(view.state.doc.toString(), s.from, s.to, event.key);
    if (!edit) return false;
    apply(edit);
    if (doubleRefKind(edit.text, edit.start, edit.end)) {
      view.dispatch({ selection: { anchor: edit.end } });
      ctx.refreshCompletion();
    }
    return true;
  };
  const changed = (update: ViewUpdate) => {
    const hasSelection = !update.state.selection.main.empty;
    setSelected(hasSelection);
    if (!hasSelection) setOverflow(false);
  };
  createEffect(() => {
    if (!overflow() || !selected()) return;
    onCleanup(registerTransientLayer({ id: layerId, root: () => overflowRef ?? null,
      trigger: () => ctx.view()?.contentDOM ?? null,
      dismiss: () => {
        const view = ctx.view(), selection = view?.state.selection;
        setOverflow(false);
        queueMicrotask(() => {
          if (!current() || !view || !selection) return;
          view.dispatch({ selection }); view.focus();
        });
        return true;
      } }));
  });
  const button = (item: SelectionAction, menu = false) => <button type="button" role={menu ? "menuitem" : undefined}
    classList={{ "sel-action-page-link": item.id === "page-link" }} title={item.title} aria-label={item.title}
    data-selection-action={item.id} onClick={() => action(item)}>{selectionActionLabel(item)}</button>;
  const Toolbar = () => <Show when={selected()}>
    <div class="sel-toolbar" classList={{ "sel-toolbar-mobile": touchPrimary() }} data-mobile-selection-toolbar={touchPrimary() ? "" : undefined}
      onPointerDown={event => event.preventDefault()} onMouseDown={event => event.preventDefault()}>
      <For each={essentialSelectionActions}>{item => button(item)}</For>
      <div class="sel-toolbar-secondary"><For each={secondarySelectionActions}>{item => button(item)}</For></div>
      <button type="button" class="sel-toolbar-more" title={t("editor-block-more-formatting")} aria-label={t("editor-block-more-formatting")}
        aria-haspopup="menu" aria-expanded={overflow()} onClick={() => setOverflow(value => !value)}>…</button>
      <Show when={!cap}><button type="button" data-selection-action="comment" title={t("comment-toolbar-shortcut")} aria-label={t("comment-toolbar")} onClick={comment}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 11.5a8.5 8.5 0 0 1-8.5 8.5H4l-3 3V11.5a10 10 0 0 1 20 0Z" /></svg>
      </button></Show>
      <Show when={overflow()}><div ref={overflowRef} class="sel-toolbar-overflow" role="menu" aria-label={t("editor-block-more-formatting")}>
        <For each={secondarySelectionActions}>{item => button(item, true)}</For>
      </div></Show>
    </div>
  </Show>;
  return { commands, keydown, changed, Toolbar };
}

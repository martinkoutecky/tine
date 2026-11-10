import { createEffect, createSignal, createUniqueId, onCleanup, Show, useContext } from "solid-js";
import { EditorView, type ViewUpdate } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { editorCompletions, propertyNameInputCaret } from "./editorCompletions";
import { cm6InputData } from "./cm6InputIntent";
import { applyCompletion, pageInsert, withRefCompletionSpace } from "./autocomplete";
import { graphOwner } from "../owned";
import { calcSource } from "./calc";
import { spaceAfterRefCompletion } from "../refCompletionSettings";
import { youtubeTimestampMacroFor } from "../components/youtubeTimestampEdit";
import { openDatePicker } from "../ui";
import { ResultWriteTargetContext, liveWriteTarget } from "../components/queryResultIdentity";
import { CaptureCtx } from "../components/Block";
import { registerTransientLayer } from "../transientLayers";
import { EditorAutocomplete } from "../components/EditorAutocomplete";
import { editorAssets } from "./editorAssets";
import { isPermittedTabGesture } from "../keybindings";
import { guardedCommands } from "./cm6Commands";
import { msg } from "../i18n";
import { pushToast } from "../toasts";

/** CM transactions own the buffer and selection; completion never writes files.
 * Reads retain the same ranking, baseline leases and durable ref publication as
 * the textarea. Only input transactions start completion, so model sync/history
 * cannot accidentally reopen a consumed trigger. */
export function cm6Completions(props: {
  id: string; surface: string; format: "md" | "org"; sheet: boolean;
  current: () => boolean; view: () => EditorView | undefined;
  visible: () => string; rawBeforeCommit: () => string;
  literal(): boolean; annotation(): boolean; languagePicker(active: boolean): void;
  completeRegion(text: string, caret: number): void;
  assets: ReturnType<typeof editorAssets>;
}) {
  const cap = useContext(CaptureCtx);
  const inResults = useContext(ResultWriteTargetContext);
  const view = () => props.view()!;
  const text = () => view().state.doc.toString();
  const caret = () => view().state.selection.main.head;
  const current = () => props.current() && !!props.view()?.dom.isConnected;
  let applying = false;
  let acListRef: HTMLDivElement | undefined;
  // Completion edits are a single CM transaction, with the caret restored before
  // any picker takes focus. The document update listener uses the ordinary commit.
  const apply = (raw: string, start: number, end = start, focus = true) => {
    if (!current()) return;
    applying = true;
    try {
      view().dispatch({ changes: { from: 0, to: view().state.doc.length, insert: raw },
        selection: { anchor: start, head: end }, userEvent: "input.complete", scrollIntoView: true });
    } finally { applying = false; }
    if (focus) view().focus();
  };
  const completion = editorCompletions({
    id: props.id, format: () => props.format, current, text, caret,
    composing: () => view().compositionStarted,
    // CM6-D9: a projected code body is code (detectTrigger owns the raw-fence rule).
    suppressTriggers: () => props.literal() || props.sheet || props.annotation() || calcSource(text()) !== null,
    keepEmptyLanguagePicker: true,
    selection: () => { const s = view().state.selection.main; return [s.anchor, s.head]; },
    rawBeforeCommit: props.rawBeforeCommit, visible: props.visible,
    apply, afterEdit: callback => { callback(); reanchor(); },
    replace: (raw, position, close) => {
      apply(raw, position); close(); props.completeRegion(raw, position);
    },
    languagePicker: props.languagePicker, closed: () => props.languagePicker(false),
    element: () => view().contentDOM,
    resultTarget: () => inResults(props.id) ? liveWriteTarget(props.id) : undefined,
    openJournal: () => openJournal(), assets: props.assets,
    quickSwitch: cap ? (query, limit) => cap.quickSwitch(query, limit) : undefined,
  });
  const { ac, acItems, acBlockState, acIndex, setAcIndex, acVisible, closeAc,
    selectAc, updateAutocomplete, selectionChanged } = completion;
  let refreshedInput: Pick<EditorState, "doc" | "selection"> | undefined;
  const refresh = () => {
    const { doc, selection } = view().state;
    refreshedInput = { doc, selection };
    completion.refresh();
  };
  const openPlanning = (kind: "scheduled" | "deadline") => {
    const rect = view().contentDOM.getBoundingClientRect();
    openDatePicker(props.id, kind, rect.left, rect.bottom + 4, inResults(props.id) ? liveWriteTarget(props.id) : undefined);
  };
  const openJournal = () => {
    const owner = graphOwner(current);
    const before = text(), position = caret();
    const rect = view().contentDOM.getBoundingClientRect();
    openDatePicker(props.id, { insertJournal: title => {
      if (!owner() || text() !== before) {
        pushToast(msg("journal-slash-the-date-was-not-inserted-because-the-graph"), "info"); return;
      }
      const insert = pageInsert(title), edit = applyCompletion(before, position, position, insert);
      const result = withRefCompletionSpace(edit.raw, edit.caret, insert, spaceAfterRefCompletion());
      apply(result.raw, result.caret, result.caret, false);
    } }, rect.left, rect.bottom + 4);
  };
  const changed = (update: ViewUpdate) => {
    if (applying || !current()) return;
    if (update.transactions.some(tr => tr.isUserEvent("input") || tr.isUserEvent("delete"))) {
      if (view().compositionStarted || update.transactions.some(tr => tr.isUserEvent("input.type.compose"))) return;
      refresh();
    } else if (update.selectionSet) selectionChanged();
  };
  const input = (event: InputEvent) => {
    if (!current() || event.isComposing || view().compositionStarted || applying) return;
    // CM's transaction listener owns native edits. The platform notification
    // may arrive before or after its DOM reconciliation; wait for that flush
    // and only handle a state that the transaction path has not refreshed.
    // A notification without a preceding transaction can open an initial trigger.
    queueMicrotask(() => {
      if (!current() || view().compositionStarted || applying) return;
      const state = view().state;
      const refreshed = refreshedInput;
      refreshedInput = undefined;
      if (refreshed?.doc === state.doc && refreshed.selection.eq(state.selection)) return;
      // Consume only the transaction's companion notification. A later standalone
      // input can refresh changed graph data even when the buffer is unchanged.
      completion.refresh();
    });
  };
  // The property-name workflow authors name|::, starting with an empty ::.
  // Adjust the input transaction itself so observers never see a stale caret.
  const inputSelection = EditorState.transactionFilter.of(tr => {
    if (props.sheet || !tr.isUserEvent("input.type") || tr.isUserEvent("input.type.compose")) return tr;
    let inserted = "";
    tr.changes.iterChanges((_from, _to, _newFrom, _newTo, text) => { inserted += text.toString(); });
    const inputData = tr.annotation(cm6InputData);
    const position = propertyNameInputCaret(tr.newDoc.toString(), tr.newSelection.main.head,
      inputData === undefined ? inserted : inputData ?? "");
    if (position !== null)
      return [tr, { selection: { anchor: position }, sequential: true }];
    return tr;
  });
  const insertTrigger = (kind: "page" | "block" | "slash") => {
    const s = view().state.selection.main, selected = text().slice(s.from, s.to);
    const page = kind === "page", slash = kind === "slash";
    const insert = slash ? "/" : (page ? "[[" : "((") + selected + (page ? "]]" : "))");
    apply(text().slice(0, s.from) + insert + text().slice(s.to), s.from + (slash ? 1 : selected.length + 2));
    void updateAutocomplete(); return true;
  };
  const commands = guardedCommands(() => current() && !view().compositionStarted, {
    "editor/open-date-picker": () => { openPlanning("scheduled"); return true; },
    "editor/insert-youtube-timestamp": () => {
      const macro = youtubeTimestampMacroFor(view().contentDOM);
      if (macro) {
        const s = view().state.selection.main, insert = macro + " ";
        closeAc(); apply(text().slice(0, s.from) + insert + text().slice(s.to), s.from + insert.length);
      }
      return true;
    },
    "editor/open-slash-menu": () => insertTrigger("slash"),
    "editor/insert-page-ref": () => insertTrigger("page"),
    "editor/insert-block-ref": () => insertTrigger("block"),
  });
  const keydown = (event: KeyboardEvent): boolean => {
    if (acVisible() && event.key === "Escape") {
      const language = ac()?.kind === "code-language", raw = text(), pos = caret();
      closeAc(); if (language) props.completeRegion(raw, pos);
      return true;
    }
    if (ac() && acItems().length) {
      const n = acItems().length;
      if (event.key === "ArrowDown") { setAcIndex((acIndex() + 1) % n); return true; }
      if (event.key === "ArrowUp") { setAcIndex((acIndex() - 1 + n) % n); return true; }
      if (event.key === "Enter" || isPermittedTabGesture(event)) { selectAc(acItems()[acIndex()]); return true; }
    }
    return false;
  };
  const [rect, setRect] = createSignal<{ left: number; top: number; bottom: number } | null>(null);
  const reanchor = () => {
    if (!props.view() || !acVisible()) return;
    const bounds = view().contentDOM.getBoundingClientRect();
    const line = view().coordsAtPos(caret());
    setRect({ left: bounds.left, top: line?.top ?? bounds.top, bottom: line?.bottom ?? bounds.bottom });
  };
  const style = (): Record<string, string> => {
    const r = rect(); if (!r) return {};
    const win = view().dom.ownerDocument.defaultView!;
    const below = win.innerHeight - r.bottom, openUp = below < 300 && r.top > below;
    const left = Math.max(8, Math.min(r.left, win.innerWidth - (acListRef?.offsetWidth || 240) - 8));
    return openUp ? { left: `${left}px`, bottom: `${win.innerHeight - r.top + 2}px` }
      : { left: `${left}px`, top: `${r.bottom + 2}px` };
  };
  createEffect(() => { ac(); if (acVisible()) queueMicrotask(() => { if (current()) reanchor(); }); });
  const layerId = `cm6-completion-${createUniqueId()}`;
  createEffect(() => {
    if (!acVisible()) return;
    onCleanup(registerTransientLayer({ id: layerId, root: () => acListRef ?? null,
      trigger: () => props.view()?.contentDOM ?? null,
      dismiss: () => { closeAc(); if (current()) view().focus(); return true; } }));
  });
  const mount = () => {
    const win = view().dom.ownerDocument.defaultView!;
    win.addEventListener("scroll", reanchor, true); win.addEventListener("resize", reanchor);
    onCleanup(() => { win.removeEventListener("scroll", reanchor, true); win.removeEventListener("resize", reanchor); });
  };
  const Popup = () => <Show when={acVisible()}><EditorAutocomplete items={acItems()} index={acIndex()} style={style()}
    listRef={element => { acListRef = element; }} select={selectAc}
    blockState={acBlockState()} retry={() => void updateAutocomplete()} /></Show>;
  const scaffold = () => completion.openLanguagePicker("", 3, false);
  return { mount, inputSelection, changed, input, keydown, commands, selectionChanged, close: closeAc, scaffold,
    pause: () => { refreshedInput = undefined; completion.pause(); }, refresh, Popup };
}

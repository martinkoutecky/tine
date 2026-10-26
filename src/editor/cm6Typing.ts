import { EditorState, StateEffect } from "@codemirror/state";
import type { EditorView, ViewUpdate } from "@codemirror/view";
import { makeOwnNumberedList, node, pinPageWhileDrafting } from "../document";
import { autoPairing, typographyMode } from "../ui";
import { autoPairEdit, fullWidthCloserEdit, fullWidthRefReplace } from "./autocomplete";
import { autoPairInsertOnInput } from "./autopair";
import { rangeInLiteral } from "./inlineLiteral";
import { calcSource } from "./calc";
import { typoTypeReplace } from "../render/typography";

const ownNumbered = StateEffect.define<null>();

/** Apply the textarea's post-input edits before CM publishes the transaction.
 * Paste, commands, deletions and IME preedit never enter single-key transforms. */
export function cm6Typing(ctx: {
  id: string; format: "md" | "org";
  current(): boolean; view(): EditorView | undefined;
  commit(text: string): void; refresh(): void; pauseCompletion(): void;
  literal(): boolean; scaffold(): void;
}) {
  let composing = false;
  let ending = false;
  let ended = false;
  let disposed = false;
  let generation = 0;
  let endValue: string | undefined;
  let releaseHold: (() => void) | undefined;
  const begin = () => {
    if (!ctx.current()) return;
    composing = true;
    ending = false;
    ended = false;
    generation++;
    endValue = undefined;
    ctx.pauseCompletion();
    releaseHold ??= pinPageWhileDrafting(() => node(ctx.id)?.page);
  };
  const active = () => composing || ending || !!ctx.view()?.compositionStarted;
  const filter = EditorState.transactionFilter.of(tr => {
    if (!tr.docChanged) return tr;
    if (!ctx.current()) return [];
    if (composing || tr.isUserEvent("input.type.compose") || !tr.isUserEvent("input.type")) return tr;
    let typed = "", count = 0;
    tr.changes.iterChanges((_a, _b, _c, _d, insert) => { count++; typed = insert.toString(); });
    if (count !== 1 || typed.length !== 1 || !tr.newSelection.main.empty) return tr;
    const text = tr.newDoc.toString(), caret = tr.newSelection.main.head;
    if (!ctx.literal() && ctx.format === "md" && text === "```" && typed === "`" && caret === 3)
      return [tr, { changes: { from: 0, to: text.length, insert: "```\n\n```" }, selection: { anchor: 3 }, sequential: true }];
    let edit = typed === "【" || typed === "（" || typed === "〈" ? fullWidthRefReplace(text, caret)
      : typed === "】" || typed === "）" ? fullWidthCloserEdit(text, caret, autoPairing()) : null;
    if (!edit) {
      if (autoPairing()) edit = autoPairInsertOnInput(text, caret, typed);
      else if (typed === "[" || typed === "]") edit = autoPairEdit(text, caret, typed);
    }
    if (!edit && !ctx.literal() && typographyMode() === "type" && calcSource(text) === null)
      edit = typoTypeReplace(text, caret, typed, (from, to) => rangeInLiteral(text, ctx.format, from, to));
    if (!ctx.literal() && typed === " " && text === "1. ")
      return [tr, { changes: { from: 0, to: text.length, insert: "" }, selection: { anchor: 0 },
        effects: ownNumbered.of(null), sequential: true }];
    return edit ? [tr, { changes: { from: 0, to: text.length, insert: edit.value },
      selection: { anchor: edit.caret }, sequential: true }] : tr;
  });
  const changed = (update: ViewUpdate) => {
    if (update.transactions.some(tr => tr.isUserEvent("input.type.compose")) && !ending) {
      // Android can reconcile the final composition mutation on a timer after
      // compositionend. It is a final flush, not a new unending preedit session.
      if (ended && !update.view.compositionStarted) end();
      else begin();
    }
    if (composing || ending || update.view.compositionStarted) return false;
    if (update.transactions.some(tr => tr.isUserEvent("input") || tr.isUserEvent("delete"))) {
      const duplicate = endValue === update.state.doc.toString();
      endValue = undefined;
      if (duplicate) return false;
    }
    if (update.transactions.some(tr => tr.effects.some(effect => effect.is(ownNumbered)))) {
      makeOwnNumberedList(ctx.id, "");
      return true;
    }
    if (update.docChanged) {
      // Hold the raw scaffold before committing, so the reactive projection
      // cannot hide its opener while the picker is being installed.
      if (!ctx.literal() && update.state.doc.toString() === "```\n\n```" && update.state.selection.main.head === 3
        && update.transactions.some(tr => tr.isUserEvent("input.type"))) ctx.scaffold();
      ctx.commit(update.state.doc.toString());
    }
    return true;
  };
  const finish = () => {
    if (!composing && !ending) return;
    const view = ctx.view();
    if (disposed || !view || !ctx.current()) {
      composing = ending = false;
      releaseHold?.(); releaseHold = undefined;
      return;
    }
    const text = view.state.doc.toString(), s = view.state.selection.main;
    const edit = s.empty && (fullWidthRefReplace(text, s.head) ?? fullWidthCloserEdit(text, s.head, autoPairing()));
    // Keep normalization and the final CM flush local until the single commit.
    if (edit) view.dispatch({ changes: { from: 0, to: text.length, insert: edit.value },
      selection: { anchor: edit.caret }, userEvent: "input.normalize" });
    composing = ending = false;
    ctx.commit(view.state.doc.toString());
    endValue = view.state.doc.toString();
    releaseHold?.(); releaseHold = undefined;
    ctx.refresh();
  };
  const end = () => {
    ended = true;
    ending = true;
    // CM's compositionend observer queues its final DOM reconciliation before
    // our handler. Finish after that flush, including an immediate blur.
    const at = generation;
    queueMicrotask(() => { if (at === generation) finish(); });
  };
  const input = (event: InputEvent) => {
    if (event.isComposing && !composing) begin();
    if (composing || ending) return false;
    const duplicate = endValue === ctx.view()?.state.doc.toString();
    endValue = undefined;
    // A platform notification can retry a refused commit without changing
    // CM's state again. Unchanged successful commits remain no-ops.
    if (!duplicate && ctx.view()) ctx.commit(ctx.view()!.state.doc.toString());
    return !duplicate;
  };
  // A plain key in the same event turn as compositionend still owns its normal
  // editor command (desktop WebKit). Native confirming/229 keys never call this.
  const beforeKey = () => { if (ending && !ctx.view()?.compositionStarted) finish(); };
  const dispose = () => { disposed = true; releaseHold?.(); releaseHold = undefined; };
  return { filter, changed, begin, end, input, active, settling: () => ending, beforeKey, dispose };
}

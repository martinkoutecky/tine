// The comment margin's visibility (GH #845), kept like wide and document mode
// (ui.ts): one device preference for every page and pane, in the same
// localStorage door with the same refused-write toast. Shown by default, so only
// the opt-out ("1") is stored. Hidden means no comment thread is drawn anywhere
// (the margin column, the inline copies, the quote highlights, the count
// markers); the comment blocks stay in the files and nothing is written to the
// graph. Callers other than views go through `commentMargin.ts`, which also moves
// a caret out of a thread being hidden.

import { createSignal } from "solid-js";
import { loadStr, saveStr } from "./ui";

const KEY = "logseq-claude.comment-margin-hidden";
const POPUP_KEY = "logseq-claude.comments-in-popup";
const [inPopup, setInPopup] = createSignal(loadStr(POPUP_KEY) === "1");
export const commentsInPopup = inPopup;

/** Viewer preference, like Wide mode: one local write, no graph I/O or IPC.
 * A refused write leaves the current display choice intact. */
export function changeCommentsInPopup(value: boolean): void {
  if (saveStr(POPUP_KEY, value ? "1" : null)) setInPopup(value);
}

const [hidden, setHidden] = createSignal(loadStr(KEY) === "1");
export const commentMarginHidden = hidden;

/** Persist and apply the margin's visibility. Returns false (and changes nothing)
 * when storage refused the preference, as for wide and document mode. */
export function persistCommentMarginHidden(value: boolean): boolean {
  if (!saveStr(KEY, value ? "1" : null)) return false;
  setHidden(value);
  return true;
}

/** Show the margin although storage may refuse: a comment just written must never
 * land in something invisible. The session shows it; the toast saveStr raised says
 * the preference was not saved. */
export function forceCommentMarginShown(): void {
  saveStr(KEY, null);
  setHidden(false);
}

// GH #896: independent of comments, shown by default like the existing marks.
// Reuse the device display-preference door; never write to the graph.
import { createSignal } from "solid-js";
import { loadStr, saveStr } from "./ui";

const KEY = "logseq-claude.author-marks-hidden";
const [hidden, setHidden] = createSignal(loadStr(KEY) === "1");
export const authorMarksHidden = hidden;

export function setAuthorMarksHidden(value: boolean): boolean {
  if (!saveStr(KEY, value ? "1" : null)) return false;
  setHidden(value);
  return true;
}
export function toggleAuthorMarks(): void { setAuthorMarksHidden(!hidden()); }

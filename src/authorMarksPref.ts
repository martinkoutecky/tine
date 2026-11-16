// GH #896: independent of comments, shown by default like the existing marks.
// Reuse the device display-preference door; never write to the graph.
import { createSignal } from "solid-js";
import { loadStr, saveStr } from "./ui";

const KEY = "logseq-claude.author-marks-hidden";
const [hidden, setHidden] = createSignal(loadStr(KEY) === "1");
export const authorMarksHidden = hidden;
// GH #892: the thread look reserves its gutter on every row, which only a stylesheet can end, so the window
// carries a class while marks are hidden (comments.css; internal, not a theme hook).
const reflect = () => { if (typeof document !== "undefined") document.documentElement.classList.toggle("tine-author-marks-off", hidden()); };
reflect();

export function setAuthorMarksHidden(value: boolean): boolean {
  if (!saveStr(KEY, value ? "1" : null)) return false;
  setHidden(value);
  reflect();
  return true;
}
export function toggleAuthorMarks(): void { setAuthorMarksHidden(!hidden()); }

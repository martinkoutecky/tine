import { threadRootOf } from "../document";

/** Annotation text never invokes an outline Enter command, in either editor. */
export function commentKey(e: KeyboardEvent, id: string, newline: () => void, leave: () => void): boolean {
  if ((e.key !== "Enter" && e.key !== "Escape") || e.isComposing || e.keyCode === 229 || threadRootOf(id) === null) return false;
  if (e.key === "Escape" || (e.key === "Enter" && (e.ctrlKey || e.metaKey))) leave();
  else if (e.key === "Enter") newline();
  else return false;
  e.preventDefault();
  return true;
}

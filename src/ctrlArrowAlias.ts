import { moveCaretRow } from "./editor/caretRows";
import { isMac } from "./nativeChrome";
import { shortcutKey } from "./shortcutKey";

/** Logseq binds Ctrl+P / Ctrl+N beside Up / Down on every platform: the editor,
 * the autocomplete popup and the date picker (og shortcut/config.cljs
 * :editor/up, :editor/down, :auto-complete/prev|next, :date-picker/prev-week|next-week;
 * GH #986). Tine answers "what does Up do" in its Up handlers, so the chord is
 * read as that key here and never gets a second copy of them: a key handler
 * calls `ctrlArrowAlias(e)` once and reads the result as it reads `e`.
 *
 * Exact modifiers only: Ctrl+Shift+P, Cmd+P and AltGr+P (Windows reports Ctrl+Alt)
 * stay what they were. The result is a view of `e`: `preventDefault`,
 * `stopPropagation` and every other member act on the real event. */
export function ctrlArrowAlias(e: KeyboardEvent): KeyboardEvent {
  if (!e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return e;
  const letter = shortcutKey(e);
  const key = letter === "p" ? "ArrowUp" : letter === "n" ? "ArrowDown" : null;
  if (!key) return e;
  const keyCode = key === "ArrowUp" ? 38 : 40;
  return new Proxy(e, {
    get(target, prop) {
      switch (prop) {
        case "key": case "code": return key;
        case "ctrlKey": return false;
        case "keyCode": case "which": return keyCode;
      }
      const value = Reflect.get(target, prop, target);
      return typeof value === "function" ? value.bind(target) : value;
    },
  });
}

/** The textarea editor's half of the alias: when the handler left the key alone (the caret is
 * not at the block's edge), macOS text fields move the caret on Ctrl+P / Ctrl+N themselves;
 * elsewhere there is no such binding, so the caret moves a row as Up would (a selection collapses
 * to its start or end) and the key is consumed. Composition and commands bound to the chord never
 * reach here: the caller keeps IME events and configured commands on the original event. */
export function ctrlArrowCaret(ta: HTMLTextAreaElement, e: KeyboardEvent, alias: KeyboardEvent): void {
  if (alias === e || isMac || e.defaultPrevented) return;
  moveCaretRow(ta, alias.key === "ArrowUp" ? -1 : 1);
  e.preventDefault();
}

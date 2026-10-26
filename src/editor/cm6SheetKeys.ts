import { EditorSelection } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { blockIsGridView, node, setRaw } from "../document";
import { startEditing } from "../editorController";
import { editorCommandFor, isPermittedTabGesture } from "../keybindings";
import type { SheetCellCtx } from "../sheet/context";
import { appendSheetCellChild } from "../sheet/mutations";
import { cellBlockId, cellOwner, cellSurfaceKey, moveCellAfterEdit, selectCellAfterEdit, selectTopRowSeamAfterEdit } from "../sheet/selection";
import { SHEET_CELL_HIDE, splitProps } from "./properties";

const blockedCommands = new Set([
  "editor/comment", "editor/indent", "editor/outdent", "editor/move-block-up", "editor/move-block-down",
  "editor/select-block-up", "editor/select-block-down", "editor/select-all",
]);

/** The same sheet/outline boundary as the ordinary editor, using CM visual rows.
 * Sheet selection and mutations remain owned by the existing sheet commands. */
export function cm6SheetKeys(id: string, cell: SheetCellCtx, commit: (text: string) => void) {
  const initial = node(id).raw;
  const grid = (key: string): string | null => blockIsGridView(key) ? key
    : node(key)?.children.find(child => blockIsGridView(child)) ?? null;
  const length = (key: string) => splitProps(node(key)?.raw ?? "", SHEET_CELL_HIDE).visible.length;
  const deepest = (key: string): string => {
    let cur = key;
    while (!grid(cur) && node(cur)?.children.length) cur = node(cur).children.at(-1)!;
    return cur;
  };
  const previous = (key: string, host: string): string | null => {
    const parent = node(key)?.parent;
    if (!parent) return null;
    const siblings = node(parent)?.children ?? [], index = siblings.indexOf(key);
    return index > 0 ? deepest(siblings[index - 1]) : parent === host ? host : parent;
  };
  const next = (key: string, host: string): string | null => {
    if (!grid(key) && node(key)?.children[0]) return node(key).children[0];
    let cur = key;
    while (cur !== host) {
      const parent = node(cur)?.parent;
      if (!parent) return null;
      const siblings = node(parent)?.children ?? [], index = siblings.indexOf(cur);
      if (index >= 0 && index + 1 < siblings.length) return siblings[index + 1];
      cur = parent;
    }
    return null;
  };
  const open = (key: string, offset: number) => startEditing(key, offset, cellOwner(cell));
  const keydown = (e: KeyboardEvent, v: EditorView): boolean => {
    const s = v.state.selection.main, raw = v.state.doc.toString();
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    if (!e.ctrlKey && !e.metaKey && e.altKey && e.key === "Enter") {
      commit(raw);
      const child = appendSheetCellChild(cellBlockId(cell) ?? id);
      if (child) open(child, 0);
      return true;
    }
    if (plain && e.key === "Enter" && !e.shiftKey) { commit(raw); selectCellAfterEdit(cell); return true; }
    if (isPermittedTabGesture(e)) { commit(raw); moveCellAfterEdit(cell, e.shiftKey ? "tab-back" : "tab-forward"); return true; }
    if (plain && e.key === "Escape") {
      if (node(id).raw !== initial) setRaw(id, initial, { timetracking: false });
      selectCellAfterEdit(cell);
      return true;
    }
    if (plain && s.empty && e.key === "Backspace" && s.head === 0) return true;
    if (plain && !e.shiftKey && s.empty) {
      if (e.key === "ArrowLeft" && s.head === 0 || e.key === "ArrowRight" && s.head === raw.length) {
        commit(raw); moveCellAfterEdit(cell, e.key === "ArrowLeft" ? "left" : "right"); return true;
      }
      if (e.key === "ArrowUp" && v.moveToLineBoundary(EditorSelection.cursor(s.head), false).head === 0) {
        commit(raw);
        const host = cellBlockId(cell), prev = host && id !== host ? previous(id, host) : null;
        if (prev) open(prev, length(prev));
        else moveCellAfterEdit(cell, "up");
        return true;
      }
      if (e.key === "ArrowDown" && v.moveToLineBoundary(EditorSelection.cursor(s.head), true).head === raw.length) {
        const nestedGrid = grid(id);
        commit(raw);
        if (nestedGrid) selectTopRowSeamAfterEdit(nestedGrid, 0, cellSurfaceKey(cell.gridId, cell.surfaceId));
        else {
          const host = cellBlockId(cell), following = host ? id === host ? node(host)?.children[0] : next(id, host) : null;
          if (following) open(following, 0);
          else moveCellAfterEdit(cell, "down");
        }
        return true;
      }
    }
    const cmd = editorCommandFor(e);
    return !!cmd && blockedCommands.has(cmd);
  };
  return { keydown, blocks: (command: string) => blockedCommands.has(command) };
}

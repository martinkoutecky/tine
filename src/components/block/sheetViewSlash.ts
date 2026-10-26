import { insertEmptyChildBlock, node as docNode, setBlockProperty, withUndoUnit } from "../../document";
import { endEdit, startEditing } from "../../editorController";

type SheetSlashView = "grid" | "table" | "board";

// Extracted unchanged so the oversized Block component has room for its
// experimental editor entry seam without growing past the source-size ratchet.
export function applySheetViewSlashAction(id: string, view: SheetSlashView): string | null {
  const node = docNode(id);
  if (!node) return null;
  let seededCellId: string | null = null;
  withUndoUnit(`sheet:view:${view}`, [node.page], () => {
    const shouldSeedGrid = view === "grid" && (docNode(id)?.children.length ?? 0) === 0;
    setBlockProperty(id, "tine.view", view);
    if (view === "board") setBlockProperty(id, "tine.group-by", "state");
    if (shouldSeedGrid) {
      const rowId = insertEmptyChildBlock(id, 0);
      if (rowId) seededCellId = insertEmptyChildBlock(rowId, 0);
    }
  });
  endEdit("select-block");
  if (seededCellId) startEditing(seededCellId, 0);
  return seededCellId;
}

// "Edit clock entries" (GH #975). The LOGBOOK drawer stays hidden while typing,
// as in Logseq; this one command opens a block's editor with its drawer in the
// text, the property session's supplied-drawer path (`showLogbook`).
import { blockWritable, node, pageByName, singleSelectedId } from "../document";
import { editingId, endEdit, startEditing } from "../editorController";
import { containingGridCell } from "../sheet/gridNeighbour";
import { switcherPluginBlock } from "../ui";
import { cm6EditorEnabled } from "./cm6Setting";
import { hasLogbookDrawer } from "./properties";

let requested: string | null = null;
let listedFor: string | null = null;

/** The command's availability: offered for the block in play as the commands
 *  are listed (the block focused when the palette opened, the edited block, else
 *  the single selected one). Notes that block, for running it after the palette
 *  has ended the edit (GH #918). */
export function clockEntriesInPlay(): boolean {
  listedFor = switcherPluginBlock()?.block.id ?? editingId() ?? singleSelectedId();
  return listedFor !== null && clockEntriesOffered(listedFor);
}

/** Run the command on the block in play now, else the one noted when listed. */
export function editClockEntriesInPlay(): void {
  const id = editingId() ?? singleSelectedId() ?? listedFor;
  listedFor = null;
  if (id) editClockEntries(id);
}

/** Whether the command applies to block `id`: a writable outline block (not a
 *  sheet cell) with a LOGBOOK drawer, edited with CodeMirror, the only editor
 *  that opens the drawer. O(block bytes + depth). */
export function clockEntriesOffered(id: string): boolean {
  const n = node(id);
  return !!n && cm6EditorEnabled() && blockWritable(id) && containingGridCell(id) === null
    && hasLogbookDrawer(n.raw, pageByName(n.page)?.format === "org" ? "org" : "md");
}

/** Open block `id`'s editor with its LOGBOOK drawer in the text, the caret on
 *  the first CLOCK line. An editor already open on it closes first, so the new
 *  one starts its session with the drawer. Does nothing where not offered. */
export function editClockEntries(id: string): void {
  if (!clockEntriesOffered(id)) return;
  if (editingId() === id) endEdit("select-block");
  if (editingId() === id) return;
  // The editor this request mounts takes it synchronously; a refused request leaves none behind.
  requested = id;
  startEditing(id);
  requested = null;
}

/** For a mounting editor: whether it opens for "Edit clock entries". */
export function clockEntriesRequested(id: string): boolean {
  return requested === id;
}

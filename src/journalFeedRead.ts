import { backend } from "./backend";
import { readOwned, type Owner } from "./owned";

/** Read an older feed window. Reference completion can replace a file-only
 * cursor while IPC waits; reread from that authoritative cursor once, keeping
 * the caller's pagination lock and owner. Missing/retired windows return null;
 * owned read errors reject. One read normally, two across that transition.
 * The caller wraps this operation in working-set generation admission. */
export async function readJournalFeedWindow(owner: Owner, limit: number, cursor: number, includeReferences: boolean,
  completed: () => { done: boolean; before: number | null } | null) {
  const result = await readOwned(owner, backend().journalFeedPage(limit, cursor, includeReferences));
  if (result.kind === "stale") return null;
  const current = !includeReferences ? completed() : null;
  if (!current) return { response: result.value, cursor };
  if (current.done || current.before === null) return null;
  cursor = current.before;
  const renewed = await readOwned(owner, backend().journalFeedPage(limit, cursor, true));
  return renewed.kind === "current" ? { response: renewed.value, cursor } : null;
}

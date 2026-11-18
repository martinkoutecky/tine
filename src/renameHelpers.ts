import { backend } from "./backend";
import { admitRead, captureTransferRollback, ensurePageLoaded, flushPage, pageByName, readPageProperty, setPageProperty } from "./document";
import { bindingOwner } from "./owned";

/** One page-property edit a rename helper makes before its request
 *  (D-20261115-HELP, D-20261116-TAGH): the backend read the file's `key` value
 *  as `before`; the helper writes `after` (null: the line goes). */
export type HelperEdit = { page: string; journal: boolean; key: string; before: string; after: string | null };

/** The helper's edits, saved; `undo` puts each page back exactly as it was
 *  before (a snapshot, not a second edit) and saves it. */
export type AppliedEdits = { undo: () => Promise<boolean> };

/** Why the edits were not made: `page` changed since the backend read it (or
 *  could not be loaded), or its save failed (then already put back). */
export type EditsRefused = { refused: "changed" | "unsaved"; page: string };

const same = (a: string | null, b: string) => (a ?? "").trim() === b.trim();

/** Make `edits` through the ordinary page-property write and save
 *  (`setPageProperty`: base-revision guard, normal save), after checking that
 *  each loaded page still reads the value the backend read (a comparison, no
 *  reread). Nothing is written unless every page is fresh. A failed save puts
 *  every page back. Cost: one page read per page not loaded, the edits, and
 *  one guarded save per page. */
export async function applyHelperEdits(edits: readonly HelperEdit[]): Promise<AppliedEdits | EditsRefused> {
  const owner = bindingOwner();
  const names: string[] = [];
  for (const edit of edits) {
    let name = pageByName(edit.page) ? edit.page : null;
    if (!name) {
      const kind = edit.journal ? "journal" : "page";
      const loaded = await admitRead(owner, () => backend().getPage(edit.page, kind), (dto) => [edit.page, ...(dto ? [dto.name] : [])], (dto) => dto && !ensurePageLoaded(dto) ? dto.name : null);
      if (loaded.kind === "stale" || !loaded.value || !pageByName(loaded.value)) return { refused: "changed", page: edit.page };
      name = loaded.value;
    }
    if (!same(readPageProperty(name, edit.key), edit.before)) return { refused: "changed", page: edit.page };
    names.push(name);
  }
  const rollbacks = names.map((name) => captureTransferRollback([name]));
  edits.forEach((edit, i) => setPageProperty(names[i], edit.key, edit.after));
  const undo = async () => {
    rollbacks.forEach((rollback) => rollback());
    const saved = await Promise.all(names.map((name) => flushPage(name)));
    return saved.every(Boolean);
  };
  for (const [i, name] of names.entries()) {
    if (!same(readPageProperty(name, edits[i].key), edits[i].after ?? "") || !(await flushPage(name))) {
      await undo();
      return { refused: "unsaved", page: edits[i].page };
    }
  }
  return { undo };
}

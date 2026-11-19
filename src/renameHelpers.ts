import { backend } from "./backend";
import { admitRead, ensurePageLoaded, flushPage, pageByName, pinPageNearViewport, readPageProperty, setPageProperty } from "./document";
import { bindingOwner } from "./owned";

/** One page-property edit a rename helper makes before its request
 *  (D-20261115-HELP, D-20261116-TAGH): the backend read the file's `key` value
 *  as `before`; the helper writes `after` (null: the line goes). */
export type HelperEdit = { page: string; journal: boolean; key: string; before: string; after: string | null };

/** The helper's edits, saved; `undo` puts back only the helper's own property
 *  values (each one only while it still holds the helper's value, so other
 *  typing survives) and saves, and does nothing once the graph binding retired.
 *  The edited pages stay in the working set (eviction pin) until `release`,
 *  which the caller calls once the request and any `undo` are over. */
export type AppliedEdits = { undo: () => Promise<boolean>; release: () => void };

/** Why the edits were not made: `page` changed since the backend read it (or
 *  could not be loaded, or the graph changed), or its save failed (then put
 *  back: `unsaved`; the put-back's own save failed too: `unrestored`). */
export type EditsRefused = { refused: "changed" | "unsaved" | "unrestored"; page: string };

const same = (a: string | null, b: string) => (a ?? "").trim() === b.trim();

/** Make `edits` through the ordinary page-property write and save
 *  (`setPageProperty`: base-revision guard, normal save). Pages not loaded are
 *  loaded first; then, with no await in between, every property is checked
 *  against the value the backend read (a comparison, no reread) and written.
 *  Nothing is written unless every page is fresh and the binding is current.
 *  The binding is checked after every save; a failed save or a retired binding
 *  puts the edits back (`undo`). Cost: one page read per page not loaded, the
 *  edits, and one guarded save per page. Every page is pinned against eviction
 *  before the first load (so loading many pages, or the request's own loads,
 *  cannot drop one); the pins go on every refusal here, else with `release`. */
export async function applyHelperEdits(edits: readonly HelperEdit[]): Promise<AppliedEdits | EditsRefused> {
  const owner = bindingOwner();
  const names = edits.map((edit) => edit.page);
  const unpins = names.map((_, i) => pinPageNearViewport(() => names[i]));
  const release = () => unpins.forEach((unpin) => unpin());
  const refuse = (refused: EditsRefused["refused"], page: string): EditsRefused => { release(); return { refused, page }; };
  try {
    for (const [i, edit] of edits.entries()) {
      if (pageByName(edit.page)) continue;
      const kind = edit.journal ? "journal" : "page";
      const loaded = await admitRead(owner, () => backend().getPage(edit.page, kind), (dto) => [edit.page, ...(dto ? [dto.name] : [])], (dto) => dto && !ensurePageLoaded(dto) ? dto.name : null);
      if (loaded.kind === "stale" || !loaded.value) return refuse("changed", edit.page);
      names[i] = loaded.value;
    }
    const holds = (i: number, value: string | null) => !!pageByName(names[i]) && same(readPageProperty(names[i], edits[i].key), value ?? "");
    const stale = edits.findIndex((edit, i) => !owner() || !holds(i, edit.before));
    if (stale >= 0) return refuse("changed", edits[stale].page);
    edits.forEach((edit, i) => setPageProperty(names[i], edit.key, edit.after));
    const undo = async () => {
      if (!owner()) return false;
      // A page no longer loaded cannot be put back: that is a failed restore.
      if (names.some((name) => !pageByName(name))) return false;
      edits.forEach((edit, i) => { if (holds(i, edit.after)) setPageProperty(names[i], edit.key, edit.before); });
      const saved = await Promise.all(names.map((name) => flushPage(name)));
      return owner() && saved.every(Boolean);
    };
    for (const [i, name] of names.entries()) {
      if (!holds(i, edits[i].after) || !(await flushPage(name)) || !owner()) {
        return refuse(await undo() ? "unsaved" : "unrestored", edits[i].page);
      }
    }
    return { undo, release };
  } catch (error) {
    release(); // a failed read or save rejects
    throw error;
  }
}

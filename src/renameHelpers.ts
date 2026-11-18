import { backend } from "./backend";
import { admitPageFile, admitRead, captureTransferRollback, dayMoveBlocks, emptyPage, ensurePageLoaded, flushPage, moveDayBlocks, pageByName, pageHeader, readPageProperty, reportPageLoadRefusal, setPageProperty } from "./document";
import { msg } from "./i18n";
import { bindingOwner, writeOwned, type Owner } from "./owned";
import { askContested, askDayMove } from "./renameHelperDialogs";
import { showRewriteRefusal } from "./renameRefusal";
import { pushToast } from "./toasts";
import type { ContestedOffer, DayMoveOffer, PageKind } from "./types";

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

/** Load `name` into the working set (a page with no file as an empty page),
 *  so its file is the one a write lands in (`admitPageFile`). False when the
 *  graph changed or another file holding the name has unsaved input (said). */
async function admit(owner: Owner, name: string, kind: PageKind): Promise<boolean> {
  if (pageByName(name)) return true;
  const refused = await admitPageFile(name, kind, owner, emptyPage(name, kind));
  if (refused === "stale") return false;
  if (refused) { reportPageLoadRefusal(refused); return false; }
  return !!pageByName(name);
}

/** The day move (D-20261111-CMDF, -MA, -MOA, D-20261115-HELP): renaming the
 *  day `day` is refused (a day keeps its date), so its notes move to the page
 *  the offer names, after the user confirms in the day-move dialog, which
 *  shows what stays on the day. The legal steps: for the day's own alias (MOA)
 *  the alias is first removed from the day's alias line (a helper edit, saved),
 *  so the new page can be opened under that name; then one cross-page block
 *  move writes the day (header kept, the alias line back, one `Moved to
 *  [[name]]` block) and the page (created if absent) as one guarded two-page
 *  operation. If the move does not happen the helper edit is put back. Null:
 *  cancelled, refused (said) or not done (said by the move). Cost: one dialog,
 *  up to two page reads, the alias edit's save, and one two-page operation. */
export async function moveDayNotes(day: string, offer: DayMoveOffer): Promise<{ moved: string; kind: PageKind } | null> {
  const owner = bindingOwner();
  if (!(await admit(owner, day, "journal")) || !owner()) return null;
  if (!dayMoveBlocks(day).length) {
    pushToast(msg("rename-day-nothing", { day }), "info");
    return null;
  }
  if (!(await askDayMove(day, offer.name, offer)) || !owner()) return null;
  const header = offer.release ? pageHeader(day) : null;
  let applied: AppliedEdits | null = null;
  if (offer.release) {
    const made = await applyHelperEdits([offer.release]);
    if ("refused" in made) {
      if (owner()) pushToast(msg(made.refused === "changed" ? "rename-helper-page-changed" : "rename-helper-unsaved", { page: made.page }), "error");
      return null;
    }
    applied = made;
  }
  const putBack = async () => {
    if (applied && !(await applied.undo()) && owner()) pushToast(msg("rename-helper-not-restored", { pages: day }), "error");
  };
  const kind: PageKind = offer.journal ? "journal" : "page";
  const name = offer.name;
  try {
    if (!(await admit(owner, name, kind)) || !owner()) { await putBack(); return null; }
    const from = pageByName(day), to = pageByName(name);
    if (!from || !to || (from.format ?? "md") !== (to.format ?? "md")) {
      await putBack();
      if (owner()) pushToast(msg("rename-refused-different-formats", { source: offer.day.path ?? day, target: offer.into?.path ?? name }), "error");
      return null;
    }
    const moved = moveDayBlocks(day, name, `Moved to [[${name}]]`, header ?? undefined);
    if (!moved.moved || !(await moved.persisted)) {
      await putBack();
      if (!moved.moved && owner()) pushToast(msg("rename-day-not-moved", { day }), "error");
      return null;
    }
  } catch (error) {
    await putBack();
    throw error;
  }
  if (owner()) pushToast(msg("rename-day-moved", { day, name }), "success");
  return { moved: name, kind };
}

/** The contested-name helper (D-20261019-M2(6), D-20261115-HELP): the dialog
 *  shows the files claiming the name; the user keeps one and each other file
 *  is merged into it by the existing file merge (blocks appended, aliases
 *  united, the file to the trash, no reference rewritten). `kept`: the file
 *  kept; `refused`: a merge's typed refusal was shown (with its "save and
 *  retry" for unsaved input, which retries that merge); null: cancelled or the
 *  graph changed. Merges done before a refusal stand, each complete. Another
 *  error propagates to the rename's caller, which reports it. Cost: one dialog
 *  and one guarded merge per other file. */
export async function mergeContested(from: string, to: string, offer: ContestedOffer): Promise<{ kept: string } | { refused: true } | null> {
  const owner = bindingOwner();
  const keep = await askContested(from, to, offer);
  if (!keep || !owner()) return null;
  for (const file of offer.files.filter((f) => f !== keep)) {
    const merge = (saveFirst?: string) => backend().mergePages(file, keep, ["insert-blocks", "delete-page"], saveFirst);
    try {
      const done = await writeOwned(owner, merge());
      if (done.kind === "stale") return null;
    } catch (error) {
      if (!owner() || !showRewriteRefusal(error, async (saveFirst) => { if (owner()) await writeOwned(owner, merge(saveFirst)); })) throw error;
      return { refused: true };
    }
  }
  return { kept: keep };
}

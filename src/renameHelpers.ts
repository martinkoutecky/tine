import { backend } from "./backend";
import { admitPageFile, admitRead, dayMovePlan, emptyPage, ensurePageLoaded, flushPage, formatOfFile, moveDayBlocks, pageByName, pinPageNearViewport, readPageProperty, reportPageLoadRefusal, setPageProperty } from "./document";
import { msg } from "./i18n";
import { bindingOwner, readOwned, writeOwned, type Owner } from "./owned";
import { askContested, askDayMove } from "./renameHelperDialogs";
import { showRewriteRefusal } from "./renameRefusal";
import { pushToast } from "./toasts";
import type { ContestedOffer, DayMoveOffer, PageKind } from "./types";

/** One page-property edit a rename helper makes before its request
 *  (D-20261115-HELP, D-20261116-TAGH): the backend read the file's `key` value
 *  as `before`; the helper writes `after` (null: the line goes). */
export type HelperEdit = { page: string; journal: boolean; key: string; before: string; after: string | null; header?: string | null };

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
 *  against the value the backend read (a comparison, no reread). Nothing is
 *  written unless every page is fresh and the binding is current. The edits
 *  are then made in order, each page checked again, written and published
 *  before the next, so a crash between saves leaves a prefix of them (the
 *  callers put an edit that adds a value before one that removes it); the
 *  binding is checked after every save. A failed save, a page changed
 *  meanwhile or a retired binding puts the made edits back (`undo`, in
 *  reverse order). Cost: one page read per page not loaded, the
 *  edits, and one guarded save per page. `header`: the page-header root
 *  (by id) the caller decided before this edit (the day move's plan), so
 *  `undo` writes the old value back into that root even when this edit
 *  emptied it; omitted, the header is found when written. Every page is
 *  pinned against eviction before the first load (so loading many pages, or
 *  the request's own loads, cannot drop one); the pins go on every refusal
 *  here, else with `release`. */
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
    // One page at a time, in order, each published before the next is edited
    // (a crash between saves, KC2SE): the tag union is on disk before the
    // source's tag line goes; the put-back runs in reverse, over the edits
    // made (`made`): a later page the helper never edited is the user's.
    let made = 0;
    const undo = async () => {
      if (!owner()) return false;
      // A page no longer loaded cannot be put back: that is a failed restore.
      if (names.slice(0, made).some((name) => !pageByName(name))) return false;
      for (let i = made - 1; i >= 0; i--) {
        if (!holds(i, edits[i].after)) continue;
        setPageProperty(names[i], edits[i].key, edits[i].before, edits[i].header);
        if (!(await flushPage(names[i])) || !owner()) return false;
      }
      return true;
    };
    for (const [i, name] of names.entries()) {
      if (!owner() || !holds(i, edits[i].before)) return refuse(await undo() ? "changed" : "unrestored", edits[i].page);
      setPageProperty(name, edits[i].key, edits[i].after);
      made = i + 1;
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

/** Load `name` into the working set as the file `at` (null: no file yet; a
 *  page with no file as an empty page, in the format of `file`, the file it
 *  will be created as), so the file a write lands in is the confirmed one
 *  (`admitPageFile`). A loaded page is reused only when it holds `at`, else
 *  its slot is admitted afresh (a twin merged away may still hold the name,
 *  DS-12). False when the graph changed or another file holding the name has
 *  unsaved input (said). */
async function admit(owner: Owner, name: string, kind: PageKind, at: string | null, file?: string): Promise<boolean> {
  const loaded = pageByName(name);
  if (loaded && (loaded.id ?? null) === at) return true;
  const refused = await admitPageFile(name, kind, owner, emptyPage(name, kind, file ? formatOfFile(file) : undefined));
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
 *  move writes the day (header kept, the released alias value back in it, one
 *  `Moved to [[name]]` block) and the page (created if absent) as one guarded
 *  two-page operation. If the move does not happen the helper edit is put
 *  back. `"again"`: the file the name now names is not the one the user
 *  confirmed (`offer.into`), so nothing moved and the rename is asked again.
 *  Null: cancelled, refused (said) or not done (said by the move). Cost: one
 *  dialog, up to two page reads, one name resolution, the alias edit's save,
 *  and one two-page operation. */
export async function moveDayNotes(day: string, offer: DayMoveOffer): Promise<{ moved: string; kind: PageKind } | "again" | null> {
  const owner = bindingOwner();
  if (!(await admit(owner, day, "journal", offer.day.path)) || !owner()) return null;
  if (!dayMovePlan(day).blocks.length) {
    pushToast(msg("rename-day-nothing", { day }), "info");
    return null;
  }
  if (!(await askDayMove(day, offer.name, offer)) || !owner()) return null;
  // The retained header is decided here, before the alias release can empty it.
  const header = dayMovePlan(day).header;
  let applied: AppliedEdits | null = null;
  if (offer.release) {
    const made = await applyHelperEdits([{ ...offer.release, header }]);
    if ("refused" in made) {
      if (!owner()) return null;
      if (made.refused === "unrestored") pushToast(msg("rename-helper-not-restored", { pages: day }), "error");
      else pushToast(msg(made.refused === "changed" ? "rename-helper-page-changed" : "rename-helper-unsaved", { page: made.page }), "error");
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
    if (!(await admit(owner, name, kind, offer.into?.path ?? null, offer.path)) || !owner()) { await putBack(); return null; }
    // The receiving file must be the one confirmed: a file, which one, or none
    // (DS-12). The store answers it; a page this window created holds no path.
    const at = await readOwned(owner, backend().resolvePage(name, kind));
    if (at.kind === "stale" || !owner()) { await putBack(); return null; }
    if ((at.value.kind === "existing" ? at.value.id : null) !== (offer.into?.path ?? null)) { await putBack(); return "again"; }
    const from = pageByName(day), to = pageByName(name);
    if (!from || !to || (from.format ?? "md") !== (to.format ?? "md")) {
      await putBack();
      if (owner()) pushToast(msg("rename-refused-different-formats", { source: offer.day.path ?? day, target: offer.into?.path ?? name }), "error");
      return null;
    }
    const moved = moveDayBlocks(day, name, `Moved to [[${name}]]`, header, offer.release ?? undefined);
    if (!moved.moved || !(await moved.persisted)) {
      await putBack();
      if (!moved.moved && owner()) pushToast(msg("rename-day-not-moved", { day }), "error");
      return null;
    }
  } catch (error) {
    await putBack();
    throw error;
  } finally {
    applied?.release(); // the day stays loaded through the move and its put-back
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

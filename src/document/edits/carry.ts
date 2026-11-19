import { leadingMarker, OPEN_MARKERS } from "../../markers";
import { doc, pageByName, setDoc, freshId } from "../model";
import { journalTitle, appNow } from "../../journal";
import { pageHeaderRoot, pagePropertyEdit, pageWritable, readPageProperty } from "./properties";
import { pushUndo } from "../history";
import { produce } from "solid-js/store";
import { reassignPage } from "./moves";
import { capturePages, persistTransfer, refuseConflictedMove } from "../host/wiring";

// ---------------------------------------------------------------------------
// Carry unfinished tasks forward (B)
// ---------------------------------------------------------------------------

function isOpenTask(id: string): boolean {
  // Leading task marker via the one markers.ts recognizer (vocabulary == lsdoc's, so
  // no disagreement) — parser-free, so carry works without the wasm renderer up.
  const m = leadingMarker(doc.byId[id]?.raw ?? "");
  return !!m && OPEN_MARKERS.has(m);
}
function subtreeHasOpenTask(id: string): boolean {
  const n = doc.byId[id];
  if (!n) return false;
  return isOpenTask(id) || n.children.some(subtreeHasOpenTask);
}
/** Collect the top-most open-task blocks in a subtree (open tasks not nested
 *  under another open task) — the pull-out unit when keepContext is off. */
function collectTopOpenTasks(id: string, acc: string[]) {
  if (isOpenTask(id)) {
    acc.push(id);
    return; // its open-task descendants travel with it
  }
  for (const c of doc.byId[id]?.children ?? []) collectTopOpenTasks(c, acc);
}

/** Carry unfinished tasks from `fromPages` into today's journal. Pages are
 *  processed in the given order and each batch is appended, so passing days
 *  newest→oldest puts the newest on top. `keepContext` true moves each top-level
 *  block that contains an open task whole; false pulls out just the open-task
 *  subtrees. Returns the number of blocks moved and whether the host took the
 *  move (a refused move is restored, so it is not reported as carried). Today +
 *  every fromPage must be loaded into the working set first. */
export function carryUnfinished(
  fromPages: string[],
  keepContext: boolean,
  header: string | null
): { moved: number; persisted: Promise<boolean> } {
  const none = { moved: 0, persisted: Promise.resolve(true) };
  const today = journalTitle(appNow());
  if (!pageWritable(today) || fromPages.some((page) => pageByName(page) && !pageWritable(page))) return none;
  type Item = { id: string; from: string; parent: string | null };
  const plan: Item[] = [];
  for (const fp of fromPages) {
    if (fp === today) continue;
    const page = pageByName(fp);
    if (!page) continue;
    if (keepContext) {
      for (const rid of page.roots) {
        if (subtreeHasOpenTask(rid)) plan.push({ id: rid, from: fp, parent: null });
      }
    } else {
      const ids: string[] = [];
      for (const rid of page.roots) collectTopOpenTasks(rid, ids);
      for (const id of ids) plan.push({ id, from: fp, parent: doc.byId[id].parent });
    }
  }
  if (!plan.length) return none;
  const sources = [...new Set(plan.map((item) => item.from))];
  if (refuseConflictedMove([today, ...sources])) return none;
  pushUndo("carry", [today, ...new Set(plan.map((i) => i.from))]);
  const before = capturePages([today, ...sources]);
  setDoc(
    produce((s) => {
      const todayPage = s.pages.find((p) => p.name === today);
      if (!todayPage) return;
      const carried: string[] = [];
      for (const item of plan) {
        if (item.parent === null) {
          const pg = s.pages.find((p) => p.name === item.from);
          if (pg) pg.roots = pg.roots.filter((x) => x !== item.id);
        } else {
          const par = s.byId[item.parent];
          if (par) par.children = par.children.filter((x) => x !== item.id);
        }
        s.byId[item.id].parent = null;
        reassignPage(s, item.id, today);
        carried.push(item.id);
      }
      // NB: only the carried task blocks are removed from the source day. Anything
      // the user left behind — finished tasks, notes, and blank spacer bullets — is
      // never touched. (A blank bullet that only *held* a carried task is likewise
      // left in place; it never had a task marker itself.)
      // Drop today's lone empty placeholder bullet so carried tasks don't sit
      // under a blank line.
      if (todayPage.roots.length === 1) {
        const only = s.byId[todayPage.roots[0]];
        if (only && only.children.length === 0 && only.raw.trim() === "") {
          delete s.byId[todayPage.roots[0]];
          todayPage.roots = [];
        }
      }
      if (header) {
        const hid = freshId();
        s.byId[hid] = { id: hid, raw: header, collapsed: false, parent: null, page: today, children: [] };
        todayPage.roots.push(hid);
      }
      todayPage.roots.push(...carried);
    })
  );
  const persisted = persistTransfer(before, ["move-blocks", "insert-blocks", "delete-blocks"], sources.map((source) => [source, today] as const));
  return { moved: plan.length, persisted };
}

// ---------------------------------------------------------------------------
// Move a day's notes to a page (D-20261111-CMDF, D-20261115-HELP)
// ---------------------------------------------------------------------------

/** The day-transfer plan, the one answer to "which root of `day` is its
 *  retained header" and "which current blocks move": the header's current
 *  children (the header itself stays), then every other root; no blocks when
 *  they are all empty (nothing to move). `header`: the header the caller
 *  decided earlier, before a helper edit could empty it so that
 *  {@link pageHeaderRoot} no longer sees it; omitted, it is decided now. It is
 *  the header only while it is the day's first root. O(roots + header children). */
export type DayMovePlan = { header: string | null; blocks: string[] };
export function dayMovePlan(day: string, header?: string | null): DayMovePlan {
  const page = pageByName(day);
  if (!page) return { header: null, blocks: [] };
  const id = header === undefined ? pageHeaderRoot(page)?.id : header;
  const kept = id && page.roots[0] === id ? id : null;
  const blocks = [...(kept ? doc.byId[kept].children : []), ...page.roots.filter((r) => r !== kept)];
  const empty = (id: string): boolean => doc.byId[id]?.raw.trim() === "" && doc.byId[id].children.every(empty);
  return { header: kept, blocks: blocks.every(empty) ? [] : blocks };
}

/** A page property value a helper took from the day before the move
 *  (`before`), to be put back inside the move while the line still reads the
 *  helper's `after` (other typing stays; D-20261111-MA, MOA). */
export type PropertyPutBack = { key: string; before: string; after: string | null };

/** Move the day's blocks ({@link dayMovePlan} with `header`) after the last
 *  block of `page` (the header's children promoted to roots) and leave `stub`
 *  as the day's last block; the day keeps its header node, with `putBack`'s
 *  value restored in it. One undo step, and one transfer through the page
 *  host: the day's and the page's new text go to disk as one guarded two-page
 *  operation, a refused one is restored (carry's `persistTransfer`), the
 *  put-back with it. A page with no file gets it from that operation. Both
 *  pages must be loaded and writable; an empty placeholder block of `page` is
 *  dropped. `persisted`: the host applied it. O(blocks of both pages): each
 *  parent's moved children are detached once. */
export function moveDayBlocks(day: string, page: string, stub: string, header?: string | null, putBack?: PropertyPutBack): { moved: number; persisted: Promise<boolean> } {
  const none = { moved: 0, persisted: Promise.resolve(false) };
  const plan = dayMovePlan(day, header);
  const ids = plan.blocks;
  if (day === page || !ids.length || !pageWritable(day) || !pageWritable(page)) return none;
  if (refuseConflictedMove([day, page])) return none;
  pushUndo("move-day", [day, page]);
  const before = capturePages([day, page]);
  const holds = putBack && (readPageProperty(day, putBack.key) ?? "").trim() === (putBack.after ?? "").trim();
  const restore = holds ? pagePropertyEdit(day, putBack.key, putBack.before, plan.header ? doc.byId[plan.header] : null) : null;
  setDoc(
    produce((s) => {
      const from = s.pages.find((p) => p.name === day);
      const to = s.pages.find((p) => p.name === page);
      if (!from || !to) return;
      restore?.(s);
      if (to.roots.length === 1) {
        const only = s.byId[to.roots[0]];
        if (only && only.children.length === 0 && only.raw.trim() === "") {
          delete s.byId[to.roots[0]];
          to.roots = [];
        }
      }
      // Every current child of the header moves, and every other root.
      if (plan.header) s.byId[plan.header].children = [];
      for (const id of ids) {
        s.byId[id].parent = null;
        reassignPage(s, id, page);
      }
      to.roots.push(...ids);
      const moving = new Set(ids);
      const stubId = freshId();
      s.byId[stubId] = { id: stubId, raw: stub, collapsed: false, parent: null, page: day, children: [] };
      from.roots = [...from.roots.filter((id) => !moving.has(id)), stubId];
    })
  );
  const persisted = persistTransfer(before, ["move-blocks", "insert-blocks", "delete-blocks"], [[day, page]]);
  return { moved: ids.length, persisted };
}

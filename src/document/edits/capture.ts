import { journalTitle, appNow } from "../../journal";
import { OUTLINE_MAX_DEPTH, outlineDepth, parseOutline, type OutlineNode } from "../../editor/outline";
import { type PageKind } from "../../types";
import { bindingOwner } from "../../owned";
import { pageByName, freshId, setDoc, doc } from "../model";
import { admitPageFile, reportPageLoadRefusal } from "../workingSet";
import { captureEmptyPage } from "../convert";
import { pageWritable } from "./properties";
import { insertOutlineAfter, deleteBlock } from "./blocks";
import { withUndoUnit } from "../history";
import { produce } from "solid-js/store";
import { markDirty, flushPage, baseRevFor } from "../save/engine";

/** Append a quick-capture (Logseq outline markdown, as produced by the capture
 *  window's editor — usually one bullet, but templates/multi-line paste can make
 *  several) at the END of today's journal, then flush immediately. This is the
 *  single writer for global quick-capture: routing through the live store (rather
 *  than a separate-process file append) means a capture can't race a main-view
 *  edit of today's journal into a conflict. Loads — or, if the day has no file
 *  yet, synthesizes — the journal first; never clobbers in-progress edits and
 *  refuses (false, with a message) when another file holding today's name has
 *  unsaved input (`admitPageFile`). Returns whether the write reached disk. */
export async function appendToTodayJournal(markdown: string): Promise<boolean> {
  return captureOutlineInto(journalTitle(appNow()), "journal", parseOutline(markdown));
}

/** What the share inbox sees of journal `day` inside the admitted read its
 *  append uses (ADR 0073): the revision the loaded copy is based on (`null`
 *  when the day has no file yet) and how many blocks anywhere on the page
 *  equal the outline being appended. */
export interface JournalAppendState { before: string | null; matches: number }

/** The gate's verdict: append now, the outline is already there (only flush),
 *  or stop without writing. */
export type JournalAppendDecision = "append" | "landed" | "abort";

/** Append `markdown` at the END of journal `day` exactly (never a recomputed
 *  "today"), through the same single writer as {@link appendToTodayJournal}.
 *  `gate` runs inside the admitted read, after the page is loaded and
 *  writable and before anything is inserted; it may await (the share inbox
 *  persists its recovery record there). If the page changed while it
 *  awaited, it is asked again with the new state, so what it recorded is
 *  what the insertion followed. "landed" flushes the page and inserts
 *  nothing. Returns what happened; "failed" covers refusals, a stale
 *  binding and a save that did not reach disk. */
export async function appendToJournalDay(
  day: string,
  markdown: string,
  gate: (state: JournalAppendState) => Promise<JournalAppendDecision>,
): Promise<"appended" | "landed" | "failed"> {
  const nodes = parseOutline(markdown);
  let landed = false;
  const ok = await captureOutlineInto(day, "journal", nodes, async () => {
    let state = journalAppendState(day, nodes);
    for (let attempt = 0; state && attempt < 3; attempt++) {
      const decision = await gate(state);
      if (decision !== "append") { landed = decision === "landed"; return decision; }
      const now = journalAppendState(day, nodes);
      if (now && now.before === state.before && now.matches === state.matches) return "append";
      state = now;
    }
    return "abort";
  });
  return ok ? (landed ? "landed" : "appended") : "failed";
}

function journalAppendState(day: string, nodes: OutlineNode[]): JournalAppendState | null {
  const page = pageByName(day);
  const before = baseRevFor(day);
  if (!page || before === undefined) return null;
  if (nodes.length !== 1) return { before, matches: 0 };
  let matches = 0;
  const visit = (ids: readonly string[]) => {
    for (const id of ids) {
      if (sameOutline(id, nodes[0])) matches++;
      visit(doc.byId[id]?.children ?? []);
    }
  };
  visit(page.roots);
  return { before, matches };
}

/** Block `id` and its subtree equal `node` exactly (raw text, child order). */
function sameOutline(id: string, node: OutlineNode): boolean {
  const block = doc.byId[id];
  if (!block || block.raw !== node.raw || block.children.length !== node.children.length) return false;
  return node.children.every((child, i) => sameOutline(block.children[i], child));
}

/** In-app quick capture into a (new or existing) named PAGE — the heading-filled
 *  branch of the journal-top capture bar. Same single-writer guarantees as
 *  {@link appendToTodayJournal}: routes through the live store + immediate flush,
 *  so it can't race a main-view edit of the same page into a conflict. */
export async function captureToPage(title: string, markdown: string): Promise<boolean> {
  const name = title.trim();
  if (!name) return false;
  return captureOutlineInto(name, "page", parseOutline(markdown));
}

/** Append outline `nodes` at the END of the named page (loaded — or synthesized
 *  if it has no file yet — first), then flush immediately. Shared by the journal
 *  append and the new-page capture; never clobbers in-progress edits and never
 *  writes into a second file holding the name. One page read. Returns whether
 *  it landed. */
async function captureOutlineInto(
  name: string,
  kind: PageKind,
  nodes: OutlineNode[],
  gate?: () => Promise<JournalAppendDecision>,
): Promise<boolean> {
  // Captured blocks land at root level, so the outline's own depth is the result's (I-22).
  if (!nodes.length || outlineDepth(nodes) > OUTLINE_MAX_DEPTH) return false;
  const owner = bindingOwner();
  // Admit the file the name resolves to. Another file holding the name is
  // replaced when it has no unsaved input; when it has, stop rather than append
  // into it: the capture would land where the feed does not show it and be
  // reported as saved (GH #254 family, master 7bd793bd0). Returning false keeps
  // the text in the capture window, which says so.
  const admitted = await admitPageFile(name, kind, owner, captureEmptyPage(name, kind));
  if (admitted === "stale") return false;
  if (admitted) {
    reportPageLoadRefusal(admitted, "Nothing was captured into it.");
    return false;
  }
  if (!pageByName(name) || !pageWritable(name)) return false;
  if (gate) {
    const decision = await gate();
    if (!owner()) return false;
    if (decision === "abort") return false;
    if (decision === "landed") return (await flushPage(name)) && owner();
  }
  // Re-read after the gate's await: the page may have been reloaded meanwhile.
  const page = pageByName(name);
  if (!page || !pageWritable(name)) return false;
  if (page.roots.length) {
    // Append after the last top-level block (end of the page).
    if (!insertOutlineAfter(page.roots[page.roots.length - 1], nodes)) return false;
  } else {
    // Empty (or brand-new) page: seed an empty anchor root, append after it, then
    // drop the anchor — reuses insertOutlineAfter's subtree creation rather than a
    // bespoke root builder. One undo unit: the anchor/insert/delete sequence used
    // to push three undo entries, so one undo left the anchor + row behind
    // (Phase-6 review finding, validated).
    const inserted = withUndoUnit("capture", [name], () => {
      const anchor = freshId();
      setDoc(
        produce((s) => {
          s.byId[anchor] = { id: anchor, raw: "", collapsed: false, parent: null, page: name, children: [] };
          s.pages[s.pages.findIndex((p) => p.name === name)].roots.push(anchor);
        })
      );
      markDirty(name, "insert-blocks");
      if (!insertOutlineAfter(anchor, nodes)) return false;
      deleteBlock(anchor);
      return true;
    });
    if (!inserted) return false;
  }
  const saved = await flushPage(name);
  return owner() && saved;
}

import { journalTitle, appNow } from "../../journal";
import { OUTLINE_MAX_DEPTH, outlineDepth, parseOutline, type OutlineNode } from "../../editor/outline";
import { type PageKind } from "../../types";
import { bindingOwner } from "../../owned";
import { pageByName, freshId, setDoc } from "../model";
import { admitPageFile, reportPageLoadRefusal } from "../workingSet";
import { captureEmptyPage } from "../convert";
import { pageWritable } from "./properties";
import { insertOutlineAfter, deleteBlock } from "./blocks";
import { withUndoUnit } from "../history";
import { produce } from "solid-js/store";
import { markDirty, flushPage } from "../host/wiring";

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
async function captureOutlineInto(name: string, kind: PageKind, nodes: OutlineNode[]): Promise<boolean> {
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

import { journalTitle } from "../../journal";
import { OUTLINE_MAX_DEPTH, outlineDepth, parseOutline, type OutlineNode } from "../../editor/outline";
import { type PageKind } from "../../types";
import { graphOwner, readOwned } from "../../owned";
import { pageByName, freshId, setDoc } from "../model";
import { backend } from "../../backend";
import { ensurePageLoaded } from "../workingSet";
import { captureEmptyPage } from "../convert";
import { pageWritable } from "./properties";
import { insertOutlineAfter, deleteBlock } from "./blocks";
import { withUndoUnit } from "../history";
import { produce } from "solid-js/store";
import { markDirty, flushPage } from "../save/engine";

/** Append a quick-capture (Logseq outline markdown, as produced by the capture
 *  window's editor — usually one bullet, but templates/multi-line paste can make
 *  several) at the END of today's journal, then flush immediately. This is the
 *  single writer for global quick-capture: routing through the live store (rather
 *  than a separate-process file append) means a capture can't race a main-view
 *  edit of today's journal into a conflict. Loads — or, if the day has no file
 *  yet, synthesizes — the journal first; never clobbers in-progress edits
 *  (`ensurePageLoaded` is a no-op when already loaded). Returns whether the write
 *  reached disk. */
export async function appendToTodayJournal(markdown: string): Promise<boolean> {
  return captureOutlineInto(journalTitle(new Date()), "journal", parseOutline(markdown));
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
 *  append and the new-page capture; never clobbers in-progress edits
 *  (`ensurePageLoaded` is a no-op when already loaded). Returns whether it landed. */
async function captureOutlineInto(name: string, kind: PageKind, nodes: OutlineNode[]): Promise<boolean> {
  // Captured blocks land at root level, so the outline's own depth is the result's (I-22).
  if (!nodes.length || outlineDepth(nodes) > OUTLINE_MAX_DEPTH) return false;
  const owner = graphOwner();
  if (!pageByName(name)) {
    const result = await readOwned(owner, backend().getPage(name, kind));
    if (result.kind === "stale") return false;
    const dto = result.value ?? captureEmptyPage(name, kind);
    ensurePageLoaded(dto);
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

// The window's adapter over the outline kernel (crates/tine-outline/src/outline.rs;
// outline-kernel DESIGN §6). The kernel owns what an outline edit does to a
// page; this module marshals the affected pages from the store, asks it, and
// applies its answer in one store mutation. Undo, dirty marking, the caret and
// toasts stay with the caller, in the order DESIGN §6 fixes: on refusal stop
// (no undo entry), pushUndo, apply, markDirty, startEditing.
import { produce } from "solid-js/store";
import type { EditKind, EditKinds } from "../editKind";
import { outlineApply } from "../render/parse";
import { graphRewriteFrozen } from "./graphRewriteState";
import { markDirty, pageFrozen } from "./host/wiring";
import { doc, keySource, keysUsed, pageByName, setDoc } from "./model";

/** Why a verb did nothing (`outline::Reason`). */
export type OutlineRefusal =
  | "depth_cap" | "cycle" | "read_only" | "frozen" | "cross_page_merge"
  | "header_root" | "no_outline_step" | "incomplete" | "unparseable";

/** One kernel verb (`outline::Verb`). Offsets are raw UTF-8 bytes. */
export type OutlineVerb =
  | { verb: "split"; key: string; raw_offset: number; force_child: boolean; children_visible: boolean; keep_start_in_scope: boolean }
  | { verb: "merge"; survivor: string; absorbed: string; absorbed_referenced: boolean }
  | { verb: "indent"; key: string }
  | { verb: "outdent"; key: string; logical: boolean }
  | { verb: "delete"; key: string }
  | { verb: "insert_empty_child"; parent: string; at: number };

interface Upsert { key: string; raw: string; children: string[]; collapsed?: boolean }
interface PagePatch { name: string; roots?: string[]; upserts: Upsert[]; deleted: string[]; kinds: EditKind[] }
export type OutlineNotice = { kind: "id_transferred"; from: string; to: string };

/** A computed, not yet applied, outline change. `editorOffset` is the focus
 *  caret in the editor's UTF-16 units (built-in properties hidden), or null to
 *  keep the caller's caret. */
export interface OutlineChange {
  pages: PagePatch[];
  focus: { key: string; editorOffset: number | null } | null;
  notices: OutlineNotice[];
  keysUsed: number;
}

/** The kernel's view of one loaded page: an arena keyed by block id. The
 *  parent links are not sent; the kernel derives them. O(page blocks). */
function outlinePage(name: string) {
  const page = pageByName(name);
  if (!page) return null;
  const blocks: Record<string, { raw: string; children: string[] }> = {};
  const pending = [...page.roots];
  while (pending.length) {
    const id = pending.pop()!;
    const node = doc.byId[id];
    if (!node) continue;
    blocks[id] = { raw: node.raw, children: [...node.children] };
    pending.push(...node.children);
  }
  const first = page.roots[0];
  return {
    name,
    format: page.format,
    pre_block: page.preBlock,
    roots: [...page.roots],
    blocks,
    header_root: first && doc.byId[first]?.originatedFromPageHeader ? first : null,
    read_only: page.readOnly || page.guide,
    frozen: graphRewriteFrozen() || pageFrozen(name),
  };
}

/** Ask the kernel for `verb` over the named pages. Pure: nothing is applied. */
export function outlineChange(verb: OutlineVerb, pageNames: readonly string[]): OutlineChange | { refused: OutlineRefusal } {
  const pages = [...new Set(pageNames)].map(outlinePage).filter((p) => p !== null);
  const answer = JSON.parse(outlineApply(JSON.stringify({ pages, verb, keys: keySource() })));
  if (answer.refused) return { refused: answer.refused as OutlineRefusal };
  const change = answer.ok as { pages: PagePatch[]; focus: { key: string; raw_offset: number | null } | null; notices: OutlineNotice[]; keys_used: number };
  return {
    pages: change.pages,
    focus: change.focus && { key: change.focus.key, editorOffset: change.focus.raw_offset === null ? null : answer.editor_offset },
    notices: change.notices,
    keysUsed: change.keys_used,
  };
}

/** Apply a computed change to the store in one mutation. A block's `parent`
 *  follows from the children lists and roots it was patched with; `collapsed`
 *  changes only where the kernel decided it (the cache rule, DESIGN §3). */
export function applyOutlineChange(change: OutlineChange): void {
  keysUsed(change.keysUsed);
  setDoc(produce((s) => {
    for (const patch of change.pages) {
      for (const key of patch.deleted) delete s.byId[key];
      for (const u of patch.upserts) {
        const node = s.byId[u.key];
        if (node) {
          if (node.raw !== u.raw) node.raw = u.raw;
          node.children = u.children;
          if (u.collapsed !== undefined) node.collapsed = u.collapsed;
        } else {
          s.byId[u.key] = { id: u.key, raw: u.raw, collapsed: u.collapsed ?? false, parent: null, page: patch.name, children: u.children };
        }
      }
      for (const u of patch.upserts) for (const child of u.children) s.byId[child].parent = u.key;
      if (patch.roots) {
        const page = s.pages.find((p) => p.name === patch.name)!;
        page.roots = patch.roots;
        for (const root of patch.roots) s.byId[root].parent = null;
      }
    }
  }));
}

/** Mark the changed page dirty with the kernel's kinds (DESIGN §6 step 6).
 *  Every M1 verb changes one page; a multi-page change also needs its
 *  `persistTransfer` (I-3), which arrives with the cross-page verbs (M3). */
export function markOutlineDirty(change: OutlineChange): void {
  const changed = change.pages.filter((p) => p.kinds.length);
  if (changed.length > 1) throw new Error("Outline change spans pages without a transfer");
  if (changed.length) markDirty(changed[0].name, changed[0].kinds as unknown as EditKinds);
}

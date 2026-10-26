// The window's adapter over the outline kernel (crates/tine-outline/src/outline.rs;
// outline-kernel DESIGN §6). The kernel owns what an outline edit does to a
// page; this module marshals the affected pages from the store, asks it, and
// applies its answer in one store mutation. Undo, dirty marking, the caret and
// toasts stay with the caller, in the order DESIGN §6 fixes: on refusal stop
// (no undo entry), pushUndo, apply, markDirty, startEditing.
import { produce, unwrap } from "solid-js/store";
import type { EditKind, EditKinds } from "../editKind";
import type { PageDto } from "../types";
import { outlineApply, outlineReads } from "../render/parse";
import { graphRewriteFrozen } from "./graphRewriteState";
import { capturePages, markDirty, pageFrozen, persistTransfer } from "./host/wiring";
import { doc, keySource, keysUsed, pageByName, setDoc } from "./model";

/** Why a verb did nothing (`outline::Reason`). */
export type OutlineRefusal =
  | "depth_cap" | "cycle" | "read_only" | "frozen" | "cross_page_merge"
  | "header_root" | "no_outline_step" | "incomplete" | "unparseable";

/** One kernel verb (`outline::Verb`). Offsets are raw UTF-8 bytes. */
export type OutlineVerb =
  | { verb: "split"; key: string; raw_offset: number; force_child: boolean; children_visible: boolean; keep_start_in_scope: boolean }
  | { verb: "merge"; survivor: string; absorbed: string; absorbed_referenced: boolean }
  | { verb: "indent"; keys: string[] }
  | { verb: "outdent"; keys: string[]; logical: boolean }
  | { verb: "delete"; keys: string[] }
  | { verb: "insert_empty_child"; parent: string; at: number }
  | { verb: "move_to"; keys: readonly string[]; to: OutlinePlace; inherit_from?: string | null }
  | { verb: "move_up_down"; keys: string[]; up: boolean; within_siblings?: boolean };

/** Where `move_to` puts the moved blocks (`outline::Place`). A slot's `index`
 *  counts the list before the move. */
export type OutlinePlace =
  | { at: "before" | "after" | "last_child"; key: string }
  | { at: "slot"; parent: string | null; page: string; index: number };

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
  /** `[source, destination]` pages blocks moved between: the host transfers. */
  transfers: [string, string][];
}

/** A block a verb reads, and whether it reads its subtree (`outline::Read`). */
interface OutlineRead { key: string; subtree: boolean }
type Arena = Record<string, { raw: string; children: readonly string[] }>;

/** How `outlineChange` marshals pages. `whole` sends every block (the oracle
 *  the read-set marshal is tested against); `fallbacks` counts asks the kernel
 *  found incomplete, which were asked again with whole pages. */
export const outlineMarshal = { whole: false, fallbacks: 0 };

/** The kernel's view of one loaded page: an arena keyed by block id. The
 *  parent links are not sent; the kernel derives them. With `reads`, only the
 *  verb's read set (DESIGN §5.3, `outline::Verb::reads`): each read block, its
 *  ancestors, the siblings beside it and each ancestor, and a whole subtree
 *  where asked, so a structural edit costs O(depth + siblings + subtree), not
 *  O(page). Read from the unwrapped store: a structural edit runs in an event
 *  handler, and reading blocks through the store's proxies cost more than the
 *  kernel (outline-kernel gate 2). The arrays are serialized before anything
 *  else runs, so they are not copied. */
function outlinePage(name: string, reads: readonly OutlineRead[] | null) {
  const page = unwrap(pageByName(name));
  if (!page) return null;
  const byId = unwrap(doc.byId);
  const blocks: Arena = {};
  // The block made from the page's header properties, wherever it now sits:
  // nothing may nest under it (M1 review F3). The kernel compares it only with
  // blocks it reads, so a partial page names it only when it holds it.
  let headerRoot: string | null = null;
  const add = (id: string): boolean => {
    if (blocks[id]) return true;
    const node = byId[id];
    if (!node || node.page !== name) return false;
    blocks[id] = { raw: node.raw, children: node.children };
    if (node.originatedFromPageHeader && headerRoot === null) headerRoot = id;
    return true;
  };
  const pending: string[] = [];
  if (reads) {
    for (const { key, subtree } of reads) {
      // The block, its ancestors, and the siblings beside each. A parent link
      // that disagrees with the children lists leaves a block without its
      // parent; the kernel refuses that page as incomplete.
      const walked = new Set<string>();
      for (let at: string | null = key; at !== null && !walked.has(at) && add(at); at = byId[at].parent) {
        walked.add(at);
        const parent = byId[at].parent;
        const list = parent === null ? page.roots : byId[parent]?.children ?? [];
        const i = list.indexOf(at);
        if (i > 0) add(list[i - 1]);
        if (i >= 0 && i + 1 < list.length) add(list[i + 1]);
      }
      if (subtree && blocks[key]) pending.push(...blocks[key].children);
    }
  } else {
    pending.push(...page.roots);
  }
  // A block listed twice is walked once; the kernel sees the second listing
  // and refuses the page as malformed.
  const walked = new Set<string>();
  while (pending.length) {
    const id = pending.pop()!;
    if (walked.has(id) || !add(id)) continue;
    walked.add(id);
    pending.push(...byId[id].children);
  }
  // The first root is always named when it is the header, even when the read
  // set left it out: the kernel holds it first (GH #210 F1), and a move into
  // root slot 0 need not read it.
  if (headerRoot === null && page.roots.length && byId[page.roots[0]]?.originatedFromPageHeader) headerRoot = page.roots[0];
  return {
    name,
    format: page.format,
    pre_block: page.preBlock,
    roots: page.roots,
    blocks,
    header_root: headerRoot,
    read_only: page.readOnly || page.guide,
    frozen: graphRewriteFrozen() || pageFrozen(name),
  };
}

/** Ask the kernel for `verb` over the named pages. Pure: nothing is applied.
 *  The pages go as the verb's read set; a kernel that finds it incomplete
 *  (a store whose parent links disagree with its children lists, or a read
 *  set that missed a block) is asked again with whole pages. */
export function outlineChange(verb: OutlineVerb, pageNames: readonly string[]): OutlineChange | { refused: OutlineRefusal } {
  const names = [...new Set(pageNames)];
  const ask = (reads: readonly OutlineRead[] | null) => {
    const pages = names.map((name) => outlinePage(name, reads)).filter((p) => p !== null);
    return JSON.parse(outlineApply(JSON.stringify({ pages, verb, keys: keySource() })));
  };
  const reads = outlineMarshal.whole ? null : (JSON.parse(outlineReads(JSON.stringify(verb))) as OutlineRead[]);
  let answer = ask(reads);
  if (reads && answer.refused === "incomplete") {
    outlineMarshal.fallbacks++;
    answer = ask(null);
  }
  if (answer.refused) return { refused: answer.refused as OutlineRefusal };
  const change = answer.ok as { pages: PagePatch[]; focus: { key: string; raw_offset: number | null } | null; notices: OutlineNotice[]; keys_used: number; transfers?: [string, string][] };
  return {
    pages: change.pages,
    focus: change.focus && { key: change.focus.key, editorOffset: change.focus.raw_offset === null ? null : answer.editor_offset },
    notices: change.notices,
    keysUsed: change.keys_used,
    transfers: change.transfers ?? [],
  };
}

/** The pages a change edits, for undo snapshots and conflict checks. */
export function outlineChangePages(change: OutlineChange): string[] {
  return change.pages.filter((p) => p.kinds.length).map((p) => p.name);
}

// The pre-change texts of a transfer's pages, captured by `applyOutlineChange`
// for `markOutlineDirty`.
const transferBefore = new WeakMap<OutlineChange, Map<string, PageDto>>();

/** Apply a computed change to the store in one mutation. A block's `parent`
 *  follows from the children lists and roots it was patched with, its `page`
 *  from the patch that lists it (a moved block is deleted from its source
 *  page's patch and upserted in its destination's); `collapsed` changes only
 *  where the kernel decided it (the cache rule, DESIGN §3). */
export function applyOutlineChange(change: OutlineChange): void {
  if (change.transfers.length) transferBefore.set(change, capturePages(outlineChangePages(change)));
  keysUsed(change.keysUsed);
  const upserted = new Set(change.pages.flatMap((p) => p.upserts.map((u) => u.key)));
  setDoc(produce((s) => {
    for (const patch of change.pages) {
      for (const key of patch.deleted) if (!upserted.has(key)) delete s.byId[key];
    }
    for (const patch of change.pages) {
      for (const u of patch.upserts) {
        const node = s.byId[u.key];
        if (node) {
          if (node.raw !== u.raw) node.raw = u.raw;
          node.children = u.children;
          if (node.page !== patch.name) node.page = patch.name;
          if (u.collapsed !== undefined) node.collapsed = u.collapsed;
        } else {
          s.byId[u.key] = { id: u.key, raw: u.raw, collapsed: u.collapsed ?? false, parent: null, page: patch.name, children: u.children };
        }
      }
    }
    for (const patch of change.pages) {
      for (const u of patch.upserts) for (const child of u.children) s.byId[child].parent = u.key;
      if (patch.roots) {
        const page = s.pages.find((p) => p.name === patch.name)!;
        page.roots = patch.roots;
        for (const root of patch.roots) s.byId[root].parent = null;
      }
    }
  }));
}

/** Save an applied change (DESIGN §6 step 6): each changed page is marked
 *  dirty with its kinds, and blocks that moved between pages go through one
 *  `persistTransfer` (I-3, STEP3 §8), which also marks the other pages. */
export function markOutlineDirty(change: OutlineChange): void {
  const changed = change.pages.filter((p) => p.kinds.length);
  if (!change.transfers.length) {
    for (const p of changed) markDirty(p.name, p.kinds as unknown as EditKinds);
    return;
  }
  const before = transferBefore.get(change);
  if (!before) throw new Error("Outline transfer saved without its applied change");
  const kinds = [...new Set(changed.flatMap((p) => p.kinds))] as unknown as EditKinds;
  void persistTransfer(before, kinds, change.transfers);
}

// The live editing tree. The frontend owns this during a session; all
// keystrokes and structural ops mutate it synchronously (zero IPC). Persistence
// is a debounced per-page save to Rust. See plan §"block editor model".
//
// Supports multiple pages at once (the journals feed): a single global `byId`
// map, each node tagged with its owning `page`, and an ordered `pages` list
// each with its own roots. A single-page route is just a feed of length one.

import { createStore, produce, unwrap } from "solid-js/store";
import { createSignal } from "solid-js";
import type { BlockDto, PageDto, PageKind } from "./types";
import type { OutlineNode } from "./editor/outline";
import { backend } from "./backend";

export interface Node {
  id: string;
  raw: string;
  collapsed: boolean;
  parent: string | null; // null = a root of its page
  page: string; // owning page name
  children: string[];
}

export interface FeedPage {
  name: string;
  kind: PageKind;
  title: string;
  preBlock: string | null;
  roots: string[];
}

interface DocState {
  byId: Record<string, Node>;
  pages: FeedPage[];
  loaded: boolean;
}

export const [doc, setDoc] = createStore<DocState>({ byId: {}, pages: [], loaded: false });

export const [editingId, setEditingId] = createSignal<string | null>(null);
const [caretTarget, setCaretTarget] = createSignal<{ id: string; offset: number } | null>(null);

export function takeCaretFor(id: string): number | null {
  const t = caretTarget();
  if (t && t.id === id) {
    setCaretTarget(null);
    return t.offset;
  }
  return null;
}

let idCounter = 0;
function freshId(): string {
  return `b${Date.now().toString(36)}-${idCounter++}`;
}

// ---------------------------------------------------------------------------
// Loading / serializing
// ---------------------------------------------------------------------------

function flatten(
  dtos: BlockDto[],
  parent: string | null,
  pageName: string,
  byId: Record<string, Node>
): string[] {
  return dtos.map((d) => {
    const childIds = flatten(d.children, d.id, pageName, byId);
    byId[d.id] = {
      id: d.id,
      raw: d.raw,
      collapsed: d.collapsed,
      parent,
      page: pageName,
      children: childIds,
    };
    return d.id;
  });
}

function toFeedPage(dto: PageDto, byId: Record<string, Node>): FeedPage {
  const roots = flatten(dto.blocks, null, dto.name, byId);
  return { name: dto.name, kind: dto.kind, title: dto.title, preBlock: dto.pre_block, roots };
}

/** Load a single page (page route). */
export function loadSingle(dto: PageDto) {
  const byId: Record<string, Node> = {};
  const fp = toFeedPage(dto, byId);
  setDoc({ byId, pages: [fp], loaded: true });
  setEditingId(null);
}

/** Load the journals feed (replaces current pages). */
export function loadFeed(dtos: PageDto[]) {
  const byId: Record<string, Node> = {};
  const pages = dtos.map((d) => toFeedPage(d, byId));
  setDoc({ byId, pages, loaded: true });
  setEditingId(null);
}

/** Append more pages to the feed (infinite scroll). */
export function appendFeed(dtos: PageDto[]) {
  setDoc(
    produce((s) => {
      for (const d of dtos) {
        if (s.pages.some((p) => p.name === d.name)) continue;
        s.pages.push(toFeedPage(d, s.byId));
      }
    })
  );
}

function toDto(id: string): BlockDto {
  const n = doc.byId[id];
  return { id: n.id, raw: n.raw, collapsed: n.collapsed, children: n.children.map(toDto) };
}

export function pageToDto(pageName: string): PageDto | null {
  const p = doc.pages.find((x) => x.name === pageName);
  if (!p) return null;
  return {
    name: p.name,
    kind: p.kind,
    title: p.title,
    pre_block: p.preBlock,
    blocks: p.roots.map(toDto),
  };
}

// ---------------------------------------------------------------------------
// Tree helpers
// ---------------------------------------------------------------------------

function rootsOf(id: string): string[] {
  const n = doc.byId[id];
  if (n.parent !== null) return doc.byId[n.parent].children;
  const p = doc.pages.find((x) => x.name === n.page);
  return p ? p.roots : [];
}

function indexInSiblings(id: string): number {
  return rootsOf(id).indexOf(id);
}

/** Visible blocks across the whole feed, in display order. */
export function visibleOrder(): string[] {
  const out: string[] = [];
  const walk = (ids: string[]) => {
    for (const id of ids) {
      out.push(id);
      const n = doc.byId[id];
      if (!n.collapsed && n.children.length) walk(n.children);
    }
  };
  for (const p of doc.pages) walk(p.roots);
  return out;
}

export function prevVisible(id: string): string | null {
  const order = visibleOrder();
  const i = order.indexOf(id);
  return i > 0 ? order[i - 1] : null;
}

export function nextVisible(id: string): string | null {
  const order = visibleOrder();
  const i = order.indexOf(id);
  return i >= 0 && i < order.length - 1 ? order[i + 1] : null;
}

export function depthOf(id: string): number {
  let d = 0;
  let p = doc.byId[id]?.parent ?? null;
  while (p !== null) {
    d++;
    p = doc.byId[p].parent;
  }
  return d;
}

// ---------------------------------------------------------------------------
// Undo / redo (snapshot-based; typing in one block coalesces to one step)
// ---------------------------------------------------------------------------

interface Snapshot {
  byId: Record<string, Node>;
  pages: FeedPage[];
}
const undoStack: Snapshot[] = [];
let redoStack: Snapshot[] = [];
let lastUndoTag: string | null = null;

function snapshot(): Snapshot {
  return {
    byId: structuredClone(unwrap(doc.byId)),
    pages: structuredClone(unwrap(doc.pages)),
  };
}

/** Record the current state for undo. `tag` coalesces consecutive typing in the
 *  same block (tag "type:<id>"); structural ops use distinct tags. */
function pushUndo(tag: string) {
  if (tag.startsWith("type:") && tag === lastUndoTag) return;
  undoStack.push(snapshot());
  if (undoStack.length > 200) undoStack.shift();
  redoStack = [];
  lastUndoTag = tag;
}

function restore(snap: Snapshot) {
  setDoc("byId", snap.byId);
  setDoc("pages", snap.pages);
}

export function undo() {
  if (!undoStack.length) return;
  redoStack.push(snapshot());
  restore(undoStack.pop()!);
  lastUndoTag = null;
  setEditingId(null);
  for (const p of doc.pages) dirty.add(p.name);
  scheduleSave();
}

export function redo() {
  if (!redoStack.length) return;
  undoStack.push(snapshot());
  restore(redoStack.pop()!);
  lastUndoTag = null;
  setEditingId(null);
  for (const p of doc.pages) dirty.add(p.name);
  scheduleSave();
}

// ---------------------------------------------------------------------------
// Mutations (each schedules a debounced save of the affected page)
// ---------------------------------------------------------------------------

const dirty = new Set<string>();
function markDirty(pageName: string) {
  dirty.add(pageName);
  scheduleSave();
}

export function setRaw(id: string, raw: string) {
  pushUndo(`type:${id}`);
  setDoc("byId", id, "raw", raw);
  markDirty(doc.byId[id].page);
}

export function startEditing(id: string, offset: number) {
  setSelAnchor(null);
  setSelFocus(null);
  setCaretTarget({ id, offset });
  setEditingId(id);
}

/** Enter: split the block at `offset`. */
export function splitBlock(id: string, offset: number) {
  pushUndo("split");
  const node = doc.byId[id];
  const before = node.raw.slice(0, offset);
  const after = node.raw.slice(offset);
  const newId = freshId();
  const pageName = node.page;

  setDoc(
    produce((s) => {
      s.byId[id].raw = before;
      const hasVisibleChildren = node.children.length > 0 && !node.collapsed;
      if (hasVisibleChildren) {
        s.byId[newId] = {
          id: newId, raw: after, collapsed: false, parent: id, page: pageName, children: [],
        };
        s.byId[id].children.unshift(newId);
      } else {
        s.byId[newId] = {
          id: newId, raw: after, collapsed: false, parent: node.parent, page: pageName, children: [],
        };
        const sibs = node.parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
          : s.byId[node.parent].children;
        sibs.splice(sibs.indexOf(id) + 1, 0, newId);
      }
    })
  );
  startEditing(newId, 0);
  markDirty(pageName);
}

/** Tab: make the block the last child of its previous sibling. */
export function indentBlock(id: string, caretOffset: number) {
  const i = indexInSiblings(id);
  if (i <= 0) return;
  pushUndo("indent");
  const sibs = rootsOf(id);
  const newParent = sibs[i - 1];
  const pageName = doc.byId[id].page;
  setDoc(
    produce((s) => {
      const arr = s.byId[id].parent === null
        ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
        : s.byId[s.byId[id].parent!].children;
      arr.splice(arr.indexOf(id), 1);
      s.byId[id].parent = newParent;
      s.byId[newParent].children.push(id);
      s.byId[newParent].collapsed = false;
    })
  );
  startEditing(id, caretOffset);
  markDirty(pageName);
}

/** Shift+Tab: move the block out to be the next sibling of its parent. */
export function outdentBlock(id: string, caretOffset: number) {
  const node = doc.byId[id];
  if (node.parent === null) return;
  pushUndo("outdent");
  const parentId = node.parent;
  const grandParent = doc.byId[parentId].parent;
  const pageName = node.page;

  setDoc(
    produce((s) => {
      const parent = s.byId[parentId];
      const idx = parent.children.indexOf(id);
      const following = parent.children.splice(idx);
      following.shift(); // drop id
      for (const f of following) s.byId[f].parent = id;
      s.byId[id].children.push(...following);
      s.byId[id].parent = grandParent;
      const gArr = grandParent === null
        ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
        : s.byId[grandParent].children;
      gArr.splice(gArr.indexOf(parentId) + 1, 0, id);
    })
  );
  startEditing(id, caretOffset);
  markDirty(pageName);
}

/** Backspace at offset 0: merge into the previous visible block (same page). */
export function mergeWithPrev(id: string): boolean {
  const prev = prevVisible(id);
  if (prev === null) return false;
  const node = doc.byId[id];
  if (doc.byId[prev].page !== node.page) return false; // don't merge across pages
  pushUndo("merge");
  const prevRaw = doc.byId[prev].raw;
  const joinOffset = prevRaw.length;
  const pageName = node.page;

  setDoc(
    produce((s) => {
      s.byId[prev].raw = prevRaw + node.raw;
      for (const c of node.children) s.byId[c].parent = prev;
      s.byId[prev].children.push(...node.children);
      const arr = node.parent === null
        ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
        : s.byId[node.parent].children;
      arr.splice(arr.indexOf(id), 1);
      delete s.byId[id];
    })
  );
  startEditing(prev, joinOffset);
  markDirty(pageName);
  return true;
}

/** Insert a parsed outline (from a paste) as siblings right after `afterId`.
 *  Returns the last top-level inserted block id (to focus). */
export function insertOutlineAfter(afterId: string, nodes: OutlineNode[]): string {
  if (!nodes.length) return afterId;
  pushUndo("paste");
  const parent = doc.byId[afterId].parent;
  const pageName = doc.byId[afterId].page;
  let lastId = afterId;
  setDoc(
    produce((s) => {
      const create = (n: OutlineNode, par: string | null): string => {
        const id = freshId();
        const childIds = n.children.map((c) => create(c, id));
        s.byId[id] = { id, raw: n.raw, collapsed: false, parent: par, page: pageName, children: childIds };
        return id;
      };
      const created = nodes.map((n) => create(n, parent));
      const sibs =
        parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
          : s.byId[parent].children;
      sibs.splice(sibs.indexOf(afterId) + 1, 0, ...created);
      lastId = created[created.length - 1];
    })
  );
  markDirty(pageName);
  return lastId;
}

const PROP_LINE = /^([A-Za-z0-9_./-]+):: ?(.*)$/;

/** Current value of a `key:: value` block property, or null. */
export function blockProperty(id: string, key: string): string | null {
  const node = doc.byId[id];
  if (!node) return null;
  for (const line of node.raw.split("\n")) {
    const m = PROP_LINE.exec(line);
    if (m && m[1] === key) return m[2].trim();
  }
  return null;
}

/** Set (or remove, when value is null) a `key:: value` block property. Property
 *  lines live after the block's content lines, matching Logseq. */
export function setBlockProperty(id: string, key: string, value: string | null) {
  const node = doc.byId[id];
  if (!node) return;
  pushUndo(`prop:${id}:${key}`);
  const lines = node.raw.split("\n").filter((l) => {
    const m = PROP_LINE.exec(l);
    return !(m && m[1] === key);
  });
  if (value !== null) lines.push(`${key}:: ${value}`);
  setDoc("byId", id, "raw", lines.join("\n"));
  markDirty(node.page);
}

/** Toggle a property: set it to `value`, or remove it if already that value. */
export function toggleBlockProperty(id: string, key: string, value: string) {
  setBlockProperty(id, key, blockProperty(id, key) === value ? null : value);
}

/** Set the block's heading level via the markdown `#` prefix (null clears it). */
export function setHeading(id: string, level: number | null) {
  const node = doc.byId[id];
  if (!node) return;
  pushUndo(`heading:${id}`);
  const lines = node.raw.split("\n");
  let first = (lines[0] ?? "").replace(/^#{1,6} /, "");
  if (level && level >= 1 && level <= 6) first = `${"#".repeat(level)} ${first}`;
  lines[0] = first;
  setDoc("byId", id, "raw", lines.join("\n"));
  markDirty(node.page);
}

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const pad2 = (n: number) => String(n).padStart(2, "0");

/** Read a block's SCHEDULED/DEADLINE date as {y,m,d} (m 0-based), or null. */
export function readSchedule(id: string, which: "scheduled" | "deadline"): { y: number; m: number; d: number } | null {
  const node = doc.byId[id];
  if (!node) return null;
  const tag = which === "scheduled" ? "SCHEDULED" : "DEADLINE";
  const m = new RegExp(`^${tag}:\\s*<(\\d{4})-(\\d{2})-(\\d{2})`, "m").exec(node.raw);
  return m ? { y: +m[1], m: +m[2] - 1, d: +m[3] } : null;
}

/** Set or clear a block's SCHEDULED/DEADLINE org-timestamp (line 2, like OG). */
export function setSchedule(
  id: string,
  which: "scheduled" | "deadline",
  date: { y: number; m: number; d: number } | null
) {
  const node = doc.byId[id];
  if (!node) return;
  pushUndo(`sched:${id}:${which}`);
  const tag = which === "scheduled" ? "SCHEDULED" : "DEADLINE";
  const lines = node.raw.split("\n").filter((l) => !new RegExp(`^${tag}:`).test(l.trim()));
  if (date) {
    const wd = WEEKDAYS[new Date(date.y, date.m, date.d).getDay()];
    const stamp = `${tag}: <${date.y}-${pad2(date.m + 1)}-${pad2(date.d)} ${wd}>`;
    lines.splice(Math.min(1, lines.length), 0, stamp);
  }
  setDoc("byId", id, "raw", lines.join("\n"));
  markDirty(node.page);
}

/** Collapse or expand a block and its entire descendant subtree. */
export function setCollapsedDeep(id: string, collapsed: boolean) {
  pushUndo("collapse-all");
  const walk = (bid: string) => {
    const n = doc.byId[bid];
    if (!n) return;
    if (n.children.length) setDoc("byId", bid, "collapsed", collapsed);
    n.children.forEach(walk);
  };
  walk(id);
  markDirty(doc.byId[id].page);
}

/** Ensure a block has a persistent `id::` uuid (assigned lazily, like OG);
 *  returns the uuid. Used to make `((uuid))` block references. */
export function ensureBlockId(id: string): string {
  const node = doc.byId[id];
  const m = /(?:^|\n)id:: *([0-9a-fA-F-]{8,})/.exec(node.raw);
  if (m) return m[1];
  const uuid = crypto.randomUUID();
  setDoc("byId", id, "raw", `${node.raw}\nid:: ${uuid}`);
  markDirty(node.page);
  return uuid;
}

/** Serialize a block and its subtree to Logseq markdown. */
export function blockSubtreeMarkdown(id: string, level = 0): string {
  const n = doc.byId[id];
  if (!n) return "";
  const tabs = "\t".repeat(level);
  const lines = n.raw.split("\n");
  const out: string[] = [];
  out.push(`${tabs}- ${lines[0] ?? ""}`.replace(/\s+$/, ""));
  for (const line of lines.slice(1)) {
    out.push(line === "" ? "" : `${tabs}  ${line}`);
  }
  for (const c of n.children) out.push(blockSubtreeMarkdown(c, level + 1));
  return out.join("\n");
}

/** Remove a block and its subtree. */
function deleteBlockInternal(id: string) {
  const node = doc.byId[id];
  if (!node) return;
  const pageName = node.page;
  setDoc(
    produce((s) => {
      const arr =
        node.parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === pageName)].roots
          : s.byId[node.parent!].children;
      arr.splice(arr.indexOf(id), 1);
      const rm = (bid: string) => {
        for (const c of s.byId[bid].children) rm(c);
        delete s.byId[bid];
      };
      rm(id);
    })
  );
  if (editingId() === id) setEditingId(null);
  markDirty(pageName);
}

export function deleteBlock(id: string) {
  if (!doc.byId[id]) return;
  pushUndo("delete");
  deleteBlockInternal(id);
}

// ---------------------------------------------------------------------------
// Multi-block selection (Escape from editing; Shift+Arrows extend) + ops
// ---------------------------------------------------------------------------

const [selAnchor, setSelAnchor] = createSignal<string | null>(null);
const [selFocus, setSelFocus] = createSignal<string | null>(null);

export function selectedIds(): string[] {
  const a = selAnchor();
  const f = selFocus();
  if (!a || !f) return [];
  const order = visibleOrder();
  let i = order.indexOf(a);
  let j = order.indexOf(f);
  if (i < 0 || j < 0) return [];
  if (i > j) [i, j] = [j, i];
  return order.slice(i, j + 1);
}
export function isSelected(id: string): boolean {
  return selectedIds().includes(id);
}
export function selectBlock(id: string) {
  setEditingId(null);
  setSelAnchor(id);
  setSelFocus(id);
}
export function clearSelection() {
  setSelAnchor(null);
  setSelFocus(null);
}
export function hasSelection(): boolean {
  return selAnchor() !== null;
}
export function moveSelection(dir: 1 | -1, extend: boolean) {
  const f = selFocus();
  if (!f) return;
  const order = visibleOrder();
  const i = order.indexOf(f);
  const ni = i + dir;
  if (ni < 0 || ni >= order.length) return;
  setSelFocus(order[ni]);
  if (!extend) setSelAnchor(order[ni]);
}

/** Top-level selected blocks (exclude those whose parent is also selected). */
function topSelected(): string[] {
  const ids = selectedIds();
  const set = new Set(ids);
  return ids.filter((id) => {
    const p = doc.byId[id]?.parent;
    return !(p && set.has(p));
  });
}

export function indentSelection() {
  const ids = topSelected();
  if (!ids.length) return;
  const first = ids[0];
  const sibs = rootsOf(first);
  const fi = sibs.indexOf(first);
  if (fi <= 0) return;
  const newParent = sibs[fi - 1];
  pushUndo("indent-sel");
  for (const id of ids) moveBlockInternal(id, newParent, doc.byId[newParent].children.length);
  setDoc("byId", newParent, "collapsed", false);
}

export function outdentSelection() {
  const ids = topSelected();
  if (!ids.length) return;
  const parentId = doc.byId[ids[0]].parent;
  if (parentId === null) return;
  const grand = doc.byId[parentId].parent;
  pushUndo("outdent-sel");
  let after = parentId;
  for (const id of ids) {
    moveBlockInternal(id, grand, indexInSiblings(after) + 1);
    after = id;
  }
}

export function deleteSelection() {
  const ids = topSelected();
  if (!ids.length) return;
  pushUndo("delete-sel");
  for (const id of ids) deleteBlockInternal(id);
  clearSelection();
}

export function selectionMarkdown(): string {
  return topSelected()
    .map((id) => blockSubtreeMarkdown(id))
    .join("\n");
}

/** Move a block to be a child of `newParent` (or root of its page) at `index`.
 *  Used by drag-and-drop. */
/** Move without pushing an undo entry (for batched selection ops). */
function moveBlockInternal(id: string, newParent: string | null, index: number) {
  const node = doc.byId[id];
  if (!node) return;
  let p = newParent;
  while (p !== null) {
    if (p === id) return;
    p = doc.byId[p].parent;
  }
  const oldPage = node.page;
  const newPage = newParent ? doc.byId[newParent].page : oldPage;
  setDoc(
    produce((s) => {
      const oldArr =
        node.parent === null
          ? s.pages[s.pages.findIndex((x) => x.name === oldPage)].roots
          : s.byId[node.parent!].children;
      const from = oldArr.indexOf(id);
      oldArr.splice(from, 1);
      s.byId[id].parent = newParent;
      const newArr =
        newParent === null
          ? s.pages[s.pages.findIndex((x) => x.name === newPage)].roots
          : s.byId[newParent].children;
      let idx = index;
      if (oldArr === newArr && from < idx) idx -= 1;
      newArr.splice(Math.max(0, Math.min(idx, newArr.length)), 0, id);
      if (newPage !== oldPage) {
        const reassign = (bid: string) => {
          s.byId[bid].page = newPage;
          s.byId[bid].children.forEach(reassign);
        };
        reassign(id);
      }
    })
  );
  markDirty(oldPage);
  if (newPage !== oldPage) markDirty(newPage);
}

export function moveBlock(id: string, newParent: string | null, index: number) {
  const node = doc.byId[id];
  if (!node) return;
  // Don't drop a block into its own descendant.
  let p = newParent;
  while (p !== null) {
    if (p === id) return;
    p = doc.byId[p].parent;
  }
  pushUndo("move");
  const oldPage = node.page;
  const newPage = newParent ? doc.byId[newParent].page : oldPage;
  setDoc(
    produce((s) => {
      const oldArr =
        node.parent === null
          ? s.pages[s.pages.findIndex((x) => x.name === oldPage)].roots
          : s.byId[node.parent!].children;
      const from = oldArr.indexOf(id);
      oldArr.splice(from, 1);
      s.byId[id].parent = newParent;
      const newArr =
        newParent === null
          ? s.pages[s.pages.findIndex((x) => x.name === newPage)].roots
          : s.byId[newParent].children;
      let idx = index;
      if (oldArr === newArr && from < idx) idx -= 1;
      newArr.splice(Math.max(0, Math.min(idx, newArr.length)), 0, id);
      // Reassign the moved subtree to the target page.
      if (newPage !== oldPage) {
        const reassign = (bid: string) => {
          s.byId[bid].page = newPage;
          s.byId[bid].children.forEach(reassign);
        };
        reassign(id);
      }
    })
  );
  markDirty(oldPage);
  if (newPage !== oldPage) markDirty(newPage);
}

/** Move a block up/down among its siblings (mod+Up/Down). Keyed <For> keeps the
 *  DOM node — so if the block is being edited, the textarea + caret survive. */
export function moveItem(id: string, dir: 1 | -1) {
  const node = doc.byId[id];
  if (!node) return;
  const sibs = rootsOf(id);
  const i = sibs.indexOf(id);
  const ni = i + dir;
  if (ni < 0 || ni >= sibs.length) return;
  pushUndo("move-item");
  setDoc(
    produce((s) => {
      const arr =
        node.parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === node.page)].roots
          : s.byId[node.parent!].children;
      arr.splice(i, 1);
      arr.splice(ni, 0, id);
    })
  );
  markDirty(node.page);
}

/** Move every top-level selected block up/down by one sibling slot, preserving
 *  selection. Used by mod+Up/Down in block-selection mode. */
export function moveSelectionItems(dir: 1 | -1) {
  const ids = topSelected();
  if (!ids.length) return;
  // Going down, move the bottom-most first so they don't collide; going up,
  // move the top-most first.
  const ordered = dir === 1 ? [...ids].reverse() : ids;
  for (const id of ordered) moveItem(id, dir);
}

export function toggleCollapse(id: string) {
  const n = doc.byId[id];
  if (n.children.length === 0) return;
  pushUndo("collapse");
  setDoc("byId", id, "collapsed", !n.collapsed);
  markDirty(n.page);
}

// ---------------------------------------------------------------------------
// Debounced persistence
// ---------------------------------------------------------------------------

let saveTimer: ReturnType<typeof setTimeout> | null = null;
export function scheduleSave() {
  if (!doc.loaded) return;
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    saveTimer = null;
    for (const name of dirty) {
      const dto = pageToDto(name);
      if (dto) void backend().savePage(dto);
    }
    dirty.clear();
  }, 400);
}

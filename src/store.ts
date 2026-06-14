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

// One edge-autoscroll helper for every pointer gesture that selects or moves
// blocks (GH #731).
//
// OG Logseq (0.10.9) has no explicit autoscroll code: its block drag-move is an
// HTML5 drag and its drag-select starts as a native text selection, and the
// browser scrolls the scroller under the pointer in both. Tine's gestures are
// pointer-driven (HTML5 DnD is unreliable in WebKitGTK; the rendered-block
// press is preventDefault-ed, so no native selection exists), so the browser
// never scrolls and a selection or move could not cross one screen. This module
// restores the OG contract with one implementation:
//
// - which gestures: any pointer gesture that calls `startEdgeAutoscroll`
//   (bullet drag-move, drag-select across blocks, shift-drag);
// - which containers: the innermost scrollable ancestor of whatever is under
//   the pointer whose edge zone holds it and that can still scroll that way, so
//   the main pane, either split pane, the right sidebar and an embed's own
//   scroller all work with no per-surface code;
// - speed: linear in how deep the pointer is in the zone, from MIN to MAX
//   px/s, full speed once the pointer is at or beyond the scroller's edge
//   (including outside the window, where the button is still held);
// - it stops on `stop()` (drop), mouseup/pointerup/pointercancel, Escape,
//   window blur and a move reporting no button held.
//
// Scrolling moves content under a still pointer, so no mousemove follows it:
// the gesture passes `onScrolled` and re-runs its hit test at the point it is
// given. That point is the pointer clamped into the active scroller (or the
// viewport), so a pointer past the edge resolves to the edge row.

import { documentOf, requestFrame, windowOf } from "./windowRealm";

type RealmTarget = Parameters<typeof windowOf>[0];

/** Height of the zone, inside each scroller edge, that scrolls. */
export const EDGE_ZONE_PX = 48;
/** The narrow zone for a gesture that has not committed yet (a press that may
 * still be a click or an in-block text selection): a jittery click near an edge
 * must not scroll the page, but pushing against the edge still does. */
export const PRESS_ZONE_PX = 12;
/** Scroll speed at the zone's inner boundary and at/after the edge (px/s). */
export const MIN_SPEED_PX_PER_S = 120;
export const MAX_SPEED_PX_PER_S = 1200;
/** The hit-test point is kept this far inside the scroller. The window's resize
 * grips (4-8 px along its edge) and the scrollbar sit over its outermost pixels,
 * and a point on them hits no block. */
export const HIT_INSET_PX = 10;
/** A frame gap longer than this (tab throttling, a stall) scrolls as this long. */
const MAX_FRAME_MS = 50;

export interface HitPoint {
  x: number;
  y: number;
}

/** What `update` returns: the point to hit-test, and whether a scroller's edge
 * zone holds the pointer (`edge`) - a gesture that cancels when released
 * outside the window can keep the raw pointer when it is false. */
export interface EdgeHit extends HitPoint {
  edge: boolean;
}

export interface EdgeAutoscroll {
  /** Report the pointer (client coordinates) from every move of the gesture.
   * Returns the point to hit-test: the pointer clamped into the scroller it is
   * pushing on, else into the viewport. A move reporting `buttons === 0` stops
   * the autoscroll (the release never reached us). */
  update(x: number, y: number, buttons?: number, zonePx?: number): EdgeHit;
  /** End this gesture's autoscroll. Idempotent; later `update`s only clamp. */
  stop(): void;
}

interface Pick {
  el: HTMLElement;
  rect: { top: number; bottom: number; left: number; right: number };
  /** -1 up, 1 down; 0 when the pointer is past the scroller but it cannot move. */
  dir: -1 | 0 | 1;
  /** 0..1 depth of the pointer into the zone. */
  depth: number;
}

interface Member {
  onScrolled: (x: number, y: number) => void;
  stopped: boolean;
  zone: number;
  /** The pane the gesture started in: the fallback scroller (see pickScroller). */
  home: HTMLElement | null;
}

interface Controller {
  doc: Document;
  members: Set<Member>;
  x: number;
  y: number;
  pick: Pick | null;
  dirty: boolean;
  cancelFrame: (() => void) | null;
  lastFrame: number | null;
  carry: number;
  teardown: () => void;
}

const controllers = new WeakMap<Document, Controller>();

function viewportOf(doc: Document): { w: number; h: number } {
  const win = windowOf(doc);
  return { w: win.innerWidth || doc.documentElement.clientWidth, h: win.innerHeight || doc.documentElement.clientHeight };
}

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), Math.max(lo, hi));

/** The point inside the viewport nearest to (x, y). */
export function viewportPoint(realm: RealmTarget, x: number, y: number): HitPoint {
  const { w, h } = viewportOf(documentOf(realm));
  return { x: clamp(x, 0, w - 1), y: clamp(y, 0, h - 1) };
}

function isScrollableY(n: HTMLElement): boolean {
  if (n.scrollHeight <= n.clientHeight) return false;
  const oy = windowOf(n).getComputedStyle(n).overflowY;
  return oy === "auto" || oy === "scroll" || oy === "overlay";
}

/** The pane a gesture that starts on `from` lives in: its outermost scrollable
 * ancestor. Resolve it at mousedown, before the press re-renders its target.
 * Pass the result as `startEdgeAutoscroll`'s `home`. */
export function scrollHome(from: EventTarget | null): HTMLElement | null {
  let home: HTMLElement | null = null;
  const doc = (from as Node | null)?.ownerDocument;
  for (let n = (from as Node | null)?.nodeType === 1 ? (from as HTMLElement) : null; n; n = n.parentElement) {
    if (doc && (n === doc.documentElement || n === doc.body)) break;
    if (isScrollableY(n)) home = n;
  }
  return home;
}

function canScroll(n: HTMLElement, dir: -1 | 1): boolean {
  return dir < 0 ? n.scrollTop > 0 : n.scrollTop + n.clientHeight < n.scrollHeight - 1;
}

/** Zone depth (0 outside, up to 1 at/after the edge) for the pointer and a
 * scroller's visible rect, with the direction it pushes. */
function zoneOf(y: number, top: number, bottom: number, zonePx: number): { dir: -1 | 1 | 0; depth: number } {
  const zone = Math.min(zonePx, (bottom - top) / 2);
  if (zone <= 0) return { dir: 0, depth: 0 };
  if (y < top + zone) return { dir: -1, depth: Math.min(1, (top + zone - y) / zone) };
  if (y > bottom - zone) return { dir: 1, depth: Math.min(1, (y - (bottom - zone)) / zone) };
  return { dir: 0, depth: 0 };
}

function pickScroller(doc: Document, x: number, y: number, zonePx: number, homes: HTMLElement[]): Pick | null {
  const vp = viewportOf(doc);
  const at = viewportPoint(doc, x, y);
  let fallback: Pick | null = null;
  const consider = (n: HTMLElement, rect: Pick["rect"]): Pick | null => {
    const { dir, depth } = zoneOf(y, rect.top, rect.bottom, zonePx);
    if (dir === 0) return null;
    if (canScroll(n, dir)) return { el: n, rect, dir, depth };
    fallback ??= { el: n, rect, dir: 0, depth };
    return null;
  };
  const rectOf = (n: HTMLElement) => {
    const r = n.getBoundingClientRect();
    return { top: Math.max(r.top, 0), bottom: Math.min(r.bottom, vp.h), left: Math.max(r.left, 0), right: Math.min(r.right, vp.w) };
  };
  // Innermost-first walk of the scrollable ancestors of `from`; `within` limits
  // the walk to scrollers that horizontally contain the pointer.
  const walk = (from: Element | null, within: boolean): Pick | null => {
    for (let el = from; el; el = el.parentElement) {
      const n = el as HTMLElement;
      if (n === doc.documentElement || n === doc.body) break;
      if (!isScrollableY(n)) continue;
      const rect = rectOf(n);
      if (within && (at.x < rect.left || at.x > rect.right)) continue;
      const hit = consider(n, rect);
      if (hit) return hit;
    }
    const root = doc.scrollingElement as HTMLElement | null;
    if (root && root.scrollHeight > root.clientHeight) return consider(root, { top: 0, bottom: vp.h, left: 0, right: vp.w });
    return null;
  };
  const found = walk(doc.elementFromPoint(at.x, at.y), false);
  if (found) return found;
  // Over chrome that is not inside any scroller (the window's resize grips along
  // its edge, a bar above the pane): the pane the gesture started in still owns
  // the pointer while it is within that pane's width.
  for (const home of homes) {
    if (!home.isConnected || !isScrollableY(home)) continue;
    const hit = walk(home, true);
    if (hit) return hit;
  }
  return fallback;
}

function homesOf(c: Controller): HTMLElement[] {
  const out: HTMLElement[] = [];
  for (const m of c.members) if (m.home) out.push(m.home);
  return out;
}

/** The widest zone any live gesture asks for. */
function zoneFor(c: Controller): number {
  let zone = 0;
  for (const m of c.members) zone = Math.max(zone, m.zone);
  return zone;
}

function hitPointFor(c: Controller): HitPoint {
  const r = c.pick?.rect;
  if (!r) return viewportPoint(c.doc, c.x, c.y);
  return {
    x: clamp(c.x, r.left + HIT_INSET_PX, r.right - HIT_INSET_PX),
    y: clamp(c.y, r.top + HIT_INSET_PX, r.bottom - HIT_INSET_PX),
  };
}

function tick(c: Controller, now: number) {
  c.cancelFrame = null;
  if (c.members.size === 0) return;
  const dt = c.lastFrame === null ? 16 : Math.min(now - c.lastFrame, MAX_FRAME_MS);
  c.lastFrame = now;
  let scrolled = false;
  if (c.dirty || c.pick) {
    c.pick = pickScroller(c.doc, c.x, c.y, zoneFor(c), homesOf(c));
    c.dirty = false;
    const p = c.pick;
    if (p && p.dir !== 0) {
      const speed = MIN_SPEED_PX_PER_S + (MAX_SPEED_PX_PER_S - MIN_SPEED_PX_PER_S) * p.depth;
      const want = (speed * dt) / 1000 * p.dir + c.carry;
      const whole = Math.trunc(want);
      c.carry = want - whole;
      if (whole !== 0) {
        const before = p.el.scrollTop;
        p.el.scrollTop = before + whole;
        scrolled = p.el.scrollTop !== before;
      }
    } else {
      c.carry = 0;
    }
  }
  if (scrolled) {
    const hit = hitPointFor(c);
    for (const m of [...c.members]) if (!m.stopped) m.onScrolled(hit.x, hit.y);
  }
  if (c.members.size === 0) return;
  if (c.pick) schedule(c);
  else c.lastFrame = null; // idle: the next update() restarts the clock
}

function schedule(c: Controller) {
  if (c.cancelFrame || c.members.size === 0) return;
  c.cancelFrame = requestFrame(c.doc, (now) => tick(c, now));
}

function controllerFor(doc: Document): Controller {
  let c = controllers.get(doc);
  if (c) return c;
  const ctl: Controller = {
    doc,
    members: new Set(),
    x: 0,
    y: 0,
    pick: null,
    dirty: false,
    cancelFrame: null,
    lastFrame: null,
    carry: 0,
    teardown: () => {},
  };
  const stopAll = () => {
    for (const m of [...ctl.members]) release(ctl, m);
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") stopAll();
  };
  const win = windowOf(doc);
  doc.addEventListener("mouseup", stopAll, true);
  doc.addEventListener("pointerup", stopAll, true);
  doc.addEventListener("pointercancel", stopAll, true);
  doc.addEventListener("keydown", onKey, true);
  win.addEventListener("blur", stopAll);
  ctl.teardown = () => {
    doc.removeEventListener("mouseup", stopAll, true);
    doc.removeEventListener("pointerup", stopAll, true);
    doc.removeEventListener("pointercancel", stopAll, true);
    doc.removeEventListener("keydown", onKey, true);
    win.removeEventListener("blur", stopAll);
  };
  controllers.set(doc, ctl);
  return ctl;
}

function release(c: Controller, m: Member) {
  m.stopped = true;
  c.members.delete(m);
  if (c.members.size > 0) return;
  c.cancelFrame?.();
  c.cancelFrame = null;
  c.pick = null;
  c.lastFrame = null;
  c.carry = 0;
  c.teardown();
  controllers.delete(c.doc);
}

/** Start edge autoscroll for one gesture in `realm`'s window. Several gestures
 * live at once (a press arms both the edit gesture and the document-level
 * selection drag): they share one scroll loop, so the speed never adds up, and
 * each is told when the content moved. `home` is the pane the gesture started
 * in (`scrollHome` at mousedown). Always `stop()` it when the gesture ends. */
export function startEdgeAutoscroll(
  realm: RealmTarget,
  onScrolled: (x: number, y: number) => void,
  home: HTMLElement | null = null,
): EdgeAutoscroll {
  const doc = documentOf(realm);
  const c = controllerFor(doc);
  const member: Member = { onScrolled, stopped: false, zone: EDGE_ZONE_PX, home };
  c.members.add(member);
  return {
    update(x, y, buttons, zonePx = EDGE_ZONE_PX) {
      if (member.stopped) return { ...viewportPoint(doc, x, y), edge: false };
      if (buttons === 0) {
        release(c, member);
        return { ...viewportPoint(doc, x, y), edge: false };
      }
      member.zone = zonePx;
      c.x = x;
      c.y = y;
      c.pick = pickScroller(doc, x, y, zoneFor(c), homesOf(c));
      c.dirty = false;
      if (c.pick) schedule(c);
      return { ...hitPointFor(c), edge: c.pick !== null };
    },
    stop() {
      if (!member.stopped) release(c, member);
    },
  };
}

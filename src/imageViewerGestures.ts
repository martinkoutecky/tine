// Image viewer gestures (GH #501): pinch-zoom, double-tap zoom, swipe between
// the page's images, swipe down to close. OG gets these from PhotoSwipe
// (extensions/lightbox.cljs, defaults - OG has no gesture code of its own);
// Tine hand-writes the small subset instead of bundling PhotoSwipe (~50 KB
// min) because the lightbox is one fixed overlay with a fixed set of verbs.
//
// This module is the pure state machine: pointer samples in, transform /
// navigation / close out. It touches no DOM, so every threshold is unit-testable
// just under and just over (src/imageViewerGestures.test.ts). Layout feel,
// momentum and the OS compositor's handling of touch-action are device-only.
//
// Thresholds are Tine's, in the PhotoSwipe spirit; none is a user-visible
// contract beyond "a deliberate gesture works, an accidental brush does not".

export const MIN_SCALE = 1;
export const MAX_SCALE = 5;
export const DOUBLE_TAP_SCALE = 2.5;
/** Movement beyond which a touch is a drag, not a tap. */
export const TAP_SLOP_PX = 10;
export const TAP_MAX_MS = 300;
/** Two taps within this window and radius are a double tap. */
export const DOUBLE_TAP_MS = 300;
export const DOUBLE_TAP_SLOP_PX = 30;
/** Axis lock for a one-finger drag at scale 1. */
export const AXIS_LOCK_PX = 10;
/** Horizontal release distance that turns the page (or a flick past FLICK_MIN_PX). */
export const NAV_SWIPE_PX = 50;
export const FLICK_PX_PER_MS = 0.4;
export const FLICK_MIN_PX = 20;
/** Downward release distance that closes the viewer (or a fast flick past CLOSE_FLICK_MIN_PX). */
export const CLOSE_SWIPE_PX = 80;
export const CLOSE_FLICK_PX_PER_MS = 0.5;
export const CLOSE_FLICK_MIN_PX = 30;
/** A release within this long of a drag/pinch swallows the browser's follow-up click. */
export const CLICK_SWALLOW_MS = 400;
const VELOCITY_WINDOW_MS = 100;

export interface Transform { scale: number; x: number; y: number }
export interface Size { w: number; h: number }

export interface ViewerHost {
  /** The stage (overlay content box). */
  box(): Size;
  /** The image as laid out at scale 1 (object-fit: contain result). */
  image(): Size;
  /** Visual state. `drag` is the live one-finger offset at scale 1 (page-turn
   *  follows x, close-drag follows y); both 0 at rest. */
  apply(t: Transform, drag: { x: number; y: number }): void;
  /** Turn the page by +1 / -1; false when there is no such image (edge). */
  step(delta: 1 | -1): boolean;
  close(): void;
}

type Mode = "idle" | "pan" | "swipe" | "pinch";
interface Pt { x: number; y: number; t: number }

export interface ImageViewerGestures {
  down(id: number, x: number, y: number, t: number): void;
  move(id: number, x: number, y: number, t: number): void;
  up(id: number, x: number, y: number, t: number): void;
  cancel(id: number): void;
  /** True when the click the browser emits after this pointer sequence must be ignored. */
  swallowClick(now: number): boolean;
  /** Back to scale 1 (new image shown). */
  reset(): void;
  transform(): Transform;
  mode(): Mode;
}

export function clampPan(t: Transform, box: Size, img: Size): Transform {
  const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, t.scale));
  if (scale <= MIN_SCALE) return { scale: MIN_SCALE, x: 0, y: 0 };
  const mx = Math.max(0, (img.w * scale - box.w) / 2);
  const my = Math.max(0, (img.h * scale - box.h) / 2);
  return { scale, x: Math.min(mx, Math.max(-mx, t.x)), y: Math.min(my, Math.max(-my, t.y)) };
}

export function createImageViewerGestures(host: ViewerHost): ImageViewerGestures {
  let tf: Transform = { scale: 1, x: 0, y: 0 };
  let mode: Mode = "idle";
  const pts = new Map<number, Pt>();
  const start = new Map<number, Pt>();
  let lockAxis: "h" | "v" | null = null;
  let dragX = 0;
  let dragY = 0;
  let samples: Pt[] = [];
  let lastTap: Pt | null = null;
  let moved = false; // the sequence was a drag/pinch (not a tap)
  let swallowUntil = -Infinity;
  // pinch bookkeeping: content point under the midpoint at pinch start
  let pinch: { dist0: number; scale0: number; cx: number; cy: number } | null = null;
  let panFrom: { tf: Transform; p: Pt } | null = null;

  const centre = (): Size => ({ w: host.box().w / 2, h: host.box().h / 2 });
  const apply = () => host.apply(tf, { x: dragX, y: dragY });
  const rel = (p: { x: number; y: number }) => ({ x: p.x - centre().w, y: p.y - centre().h });

  function beginPinch() {
    const [a, b] = [...pts.values()];
    const mid = rel({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });
    pinch = {
      dist0: Math.max(1, Math.hypot(a.x - b.x, a.y - b.y)),
      scale0: tf.scale,
      cx: (mid.x - tf.x) / tf.scale,
      cy: (mid.y - tf.y) / tf.scale,
    };
    mode = "pinch";
    dragX = dragY = 0;
    lockAxis = null;
    moved = true;
  }

  function velocity(): { vx: number; vy: number } {
    const last = samples[samples.length - 1];
    if (!last) return { vx: 0, vy: 0 };
    const first = samples.find((s) => last.t - s.t <= VELOCITY_WINDOW_MS) ?? last;
    const dt = last.t - first.t;
    if (dt <= 0) return { vx: 0, vy: 0 };
    return { vx: (last.x - first.x) / dt, vy: (last.y - first.y) / dt };
  }

  function settleScale() {
    tf = clampPan(tf, host.box(), host.image());
    apply();
  }

  function doubleTapAt(p: Pt) {
    if (tf.scale > MIN_SCALE) {
      tf = { scale: 1, x: 0, y: 0 };
    } else {
      const r = rel(p);
      const s = DOUBLE_TAP_SCALE;
      tf = clampPan({ scale: s, x: r.x * (1 - s), y: r.y * (1 - s) }, host.box(), host.image());
    }
    apply();
  }

  return {
    down(id, x, y, t) {
      const p = { x, y, t };
      if (pts.size === 0) {
        moved = false;
        samples = [p];
        lockAxis = null;
        dragX = dragY = 0;
      }
      pts.set(id, p);
      start.set(id, p);
      if (pts.size === 2) {
        beginPinch();
      } else if (pts.size === 1) {
        mode = tf.scale > MIN_SCALE ? "pan" : "swipe";
        panFrom = { tf: { ...tf }, p };
      }
    },

    move(id, x, y, t) {
      const prev = pts.get(id);
      if (!prev) return;
      const p = { x, y, t };
      pts.set(id, p);
      if (mode === "pinch" && pts.size >= 2 && pinch) {
        const [a, b] = [...pts.values()];
        const dist = Math.hypot(a.x - b.x, a.y - b.y);
        // Allow a little overshoot below 1 / above MAX so the release can settle.
        const scale = Math.min(MAX_SCALE * 1.2, Math.max(MIN_SCALE * 0.8, (pinch.scale0 * dist) / pinch.dist0));
        const mid = rel({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });
        tf = { scale, x: mid.x - pinch.cx * scale, y: mid.y - pinch.cy * scale };
        apply();
        return;
      }
      const s0 = start.get(id)!;
      const dx = x - s0.x;
      const dy = y - s0.y;
      if (!moved && Math.hypot(dx, dy) > TAP_SLOP_PX) moved = true;
      samples.push(p);
      if (samples.length > 16) samples = samples.slice(-16);
      if (mode === "pan" && panFrom) {
        tf = clampPan({ scale: panFrom.tf.scale, x: panFrom.tf.x + (x - panFrom.p.x), y: panFrom.tf.y + (y - panFrom.p.y) }, host.box(), host.image());
        apply();
      } else if (mode === "swipe") {
        if (!lockAxis) {
          if (Math.hypot(dx, dy) <= AXIS_LOCK_PX) return;
          lockAxis = Math.abs(dx) >= Math.abs(dy) ? "h" : "v";
        }
        if (lockAxis === "h") dragX = dx;
        else dragY = Math.max(0, dy); // only downward follows the finger
        apply();
      }
    },

    up(id, x, y, t) {
      if (!pts.has(id)) return;
      pts.set(id, { x, y, t });
      const s0 = start.get(id)!;
      const wasMode = mode;
      const dx = x - s0.x;
      const dy = y - s0.y;
      const v = velocity();
      pts.delete(id);
      start.delete(id);
      if (wasMode === "pinch") {
        if (pts.size < 2) {
          pinch = null;
          settleScale();
          // The remaining finger must not turn into a stray pan/tap.
          mode = "idle";
          pts.clear();
          start.clear();
          swallowUntil = t + CLICK_SWALLOW_MS;
        }
        return;
      }
      if (pts.size > 0) return;
      mode = "idle";
      const tapped = !moved && t - s0.t <= TAP_MAX_MS;
      if (tapped) {
        if (lastTap && t - lastTap.t <= DOUBLE_TAP_MS && Math.hypot(x - lastTap.x, y - lastTap.y) <= DOUBLE_TAP_SLOP_PX) {
          lastTap = null;
          doubleTapAt({ x, y, t });
          swallowUntil = t + CLICK_SWALLOW_MS;
        } else {
          lastTap = { x, y, t };
        }
        dragX = dragY = 0;
        return;
      }
      lastTap = null;
      if (moved) swallowUntil = t + CLICK_SWALLOW_MS;
      if (wasMode === "swipe") {
        const wasAxis = lockAxis;
        const commitH = Math.abs(dx) >= NAV_SWIPE_PX || (Math.abs(v.vx) >= FLICK_PX_PER_MS && Math.abs(dx) >= FLICK_MIN_PX);
        const commitV = dy >= CLOSE_SWIPE_PX || (v.vy >= CLOSE_FLICK_PX_PER_MS && dy >= CLOSE_FLICK_MIN_PX);
        dragX = dragY = 0;
        if (wasAxis === "h" && commitH) {
          // Page turned (or at the edge: snap back below).
          const dir: 1 | -1 = dx < 0 ? 1 : -1;
          if (host.step(dir)) tf = { scale: 1, x: 0, y: 0 };
        } else if (wasAxis === "v" && commitV) {
          apply();
          host.close();
          return;
        }
        apply();
        return;
      }
      if (wasMode === "pan") apply();
    },

    cancel(id) {
      pts.delete(id);
      start.delete(id);
      if (pts.size === 0) {
        mode = "idle";
        pinch = null;
        dragX = dragY = 0;
        tf = clampPan(tf, host.box(), host.image());
        apply();
      }
    },

    swallowClick(now) {
      return now <= swallowUntil;
    },
    reset() {
      tf = { scale: 1, x: 0, y: 0 };
      dragX = dragY = 0;
      lastTap = null;
      apply();
    },
    transform: () => tf,
    mode: () => mode,
  };
}

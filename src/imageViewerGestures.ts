// Image viewer gestures (GH #501): pinch-zoom, double-tap zoom, swipe between
// the page's images, swipe to close, tap toggles the controls. OG gets these
// from PhotoSwipe (extensions/lightbox.cljs passes only dataSource, pswpModule
// and showHideAnimationType, so every gesture is a PhotoSwipe 5 default; the
// lockfile resolves 5.4.4). Tine hand-writes the small subset instead of
// bundling PhotoSwipe (~50 KB min) because the lightbox is one fixed overlay
// with a fixed set of verbs. The rules below are PhotoSwipe's, ported from
// its source (dist/photoswipe.esm.js: Gestures, DragHandler, TapHandler):
//   - touch tap (after the 300 ms double-tap wait) runs `tapAction`
//     "toggle-controls", on the image and on the backdrop alike; a touch never
//     closes by tapping (the mouse click path, imageClickAction / bgClickAction,
//     stays in Toasts.tsx);
//   - a vertical drag at fit zoom closes in EITHER direction: the image follows
//     the finger with VERTICAL_DRAG_FRICTION and closes when the projected
//     release position passes MIN_RATIO_TO_CLOSE of a third of the viewport;
//   - a horizontal drag at fit zoom, and a horizontal drag that STARTED at the
//     edge of a zoomed image and continues past it (allowPanToNext), moves the
//     page strip; release turns the page on PhotoSwipe's speed / half-width rule.
//
// This module is the pure state machine: pointer samples in, transform /
// navigation / close out. It touches no DOM, so every threshold is unit-testable
// just under and just over (src/imageViewerGestures.test.ts). Layout feel,
// momentum, the neighbouring page sliding into view and the OS compositor's
// handling of touch-action are device-only; Tine shows no neighbour image while
// dragging and has no fling after a zoomed pan.

export const MIN_SCALE = 1;
export const MAX_SCALE = 5;
export const DOUBLE_TAP_SCALE = 2.5;
/** Movement beyond which a touch is a drag, not a tap. */
export const TAP_SLOP_PX = 10;
export const TAP_MAX_MS = 300;
/** PhotoSwipe DOUBLE_TAP_DELAY: a single tap waits this long for a second one. */
export const DOUBLE_TAP_MS = 300;
/** PhotoSwipe MIN_TAP_DISTANCE: the second tap must land strictly nearer than this. */
export const DOUBLE_TAP_SLOP_PX = 25;
/** PhotoSwipe AXIS_SWIPE_HYSTERISIS: travel before a drag picks its axis. */
export const AXIS_LOCK_PX = 10;
/** PhotoSwipe VERTICAL_DRAG_FRICTION: the image moves this fraction of the finger. */
export const VERTICAL_DRAG_FRICTION = 0.6;
/** PhotoSwipe MIN_RATIO_TO_CLOSE (of a third of the viewport height). */
export const MIN_RATIO_TO_CLOSE = 0.4;
/** PhotoSwipe project(v, 0.995): where a release at velocity v (px/ms) would coast to. */
export const PROJECT_MS = 0.995 / (1 - 0.995);
/** PhotoSwipe MIN_NEXT_SLIDE_SPEED (px/ms). */
export const MIN_NEXT_SLIDE_SPEED = 0.5;
/** A slow release turns the page once more than this much of the viewport width is shifted. */
export const NAV_HALF_RATIO = 0.5;
/** ...and "slow" means not faster than this against the turn (px/ms). */
export const NAV_SLOW_SPEED = 0.1;
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
  /** A confirmed single tap (PhotoSwipe tapAction "toggle-controls"). */
  tap(): void;
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
  /** Back to scale 1 (new image shown); drops a pending single tap. */
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
  let tapTimer: ReturnType<typeof setTimeout> | null = null;
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

  const clearTapTimer = () => {
    if (tapTimer !== null) clearTimeout(tapTimer);
    tapTimer = null;
  };
  /** Pan bounds at the current scale (the clamp's, so the two cannot disagree). */
  const panRange = () => {
    const c = clampPan({ scale: tf.scale, x: 1e9, y: 1e9 }, host.box(), host.image());
    return { mx: c.x, my: c.y };
  };

  /** PhotoSwipe DragHandler.change for one finger on a zoomed image: pan, and
   *  past an edge the drag STARTED at, move the page strip (allowPanToNext). */
  function panStep(stepX: number, stepY: number) {
    const { mx, my } = panRange();
    // PhotoSwipe does not pan vertically while the strip is shifted.
    if (dragX === 0) tf = { ...tf, y: Math.min(my, Math.max(-my, tf.y + stepY)) };
    if (lockAxis !== "h") {
      tf = { ...tf, x: Math.min(mx, Math.max(-mx, tf.x + stepX)) };
    } else if (dragX !== 0) {
      // A shifted strip is brought back to rest before the image pans again.
      dragX = dragX > 0 ? Math.max(0, dragX + stepX) : Math.min(0, dragX + stepX);
    } else {
      const nx = tf.x + stepX;
      const from = panFrom?.tf.x ?? tf.x;
      if (stepX > 0 && nx > mx && from >= mx) dragX += stepX;
      else if (stepX < 0 && nx < -mx && from <= -mx) dragX += stepX;
      else tf = { ...tf, x: Math.min(mx, Math.max(-mx, nx)) };
    }
    apply();
  }

  /** PhotoSwipe DragHandler.end for a shifted strip: which page, if any. */
  function pageTurn(shift: number, vx: number): 1 | -1 | 0 {
    const ratio = shift / Math.max(1, host.box().w);
    if ((vx < -MIN_NEXT_SLIDE_SPEED && ratio < 0) || (vx < NAV_SLOW_SPEED && ratio < -NAV_HALF_RATIO)) return 1;
    if ((vx > MIN_NEXT_SLIDE_SPEED && ratio > 0) || (vx > -NAV_SLOW_SPEED && ratio > NAV_HALF_RATIO)) return -1;
    return 0;
  }

  /** PhotoSwipe's vertical-drag close test at release: the frictioned offset
   *  plus where the release velocity would coast, past MIN_RATIO_TO_CLOSE of a
   *  third of the viewport, in the direction the image already moved. */
  function dragClosesViewer(pan: number, vy: number): boolean {
    const third = Math.max(1, host.box().h) / 3;
    const projected = (pan + vy * PROJECT_MS) / third;
    return (pan < 0 && projected < -MIN_RATIO_TO_CLOSE) || (pan > 0 && projected > MIN_RATIO_TO_CLOSE);
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
      if (!lockAxis && Math.hypot(dx, dy) > AXIS_LOCK_PX) lockAxis = Math.abs(dx) >= Math.abs(dy) ? "h" : "v";
      if (mode === "pan") {
        panStep(x - prev.x, y - prev.y);
      } else if (mode === "swipe" && lockAxis) {
        if (lockAxis === "h") dragX = dx;
        else dragY = dy * VERTICAL_DRAG_FRICTION; // both directions follow the finger
        apply();
      }
    },

    up(id, x, y, t) {
      if (!pts.has(id)) return;
      pts.set(id, { x, y, t });
      const s0 = start.get(id)!;
      const wasMode = mode;
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
        dragX = dragY = 0;
        if (lastTap && tapTimer !== null && Math.hypot(x - lastTap.x, y - lastTap.y) < DOUBLE_TAP_SLOP_PX) {
          clearTapTimer();
          lastTap = null;
          doubleTapAt({ x, y, t });
          swallowUntil = t + CLICK_SWALLOW_MS;
        } else {
          // A far-away second tap does not cancel the first: it fires now and
          // the new one starts its own wait.
          if (tapTimer !== null) { clearTapTimer(); host.tap(); }
          lastTap = { x, y, t };
          tapTimer = setTimeout(() => { tapTimer = null; lastTap = null; host.tap(); }, DOUBLE_TAP_MS);
        }
        return;
      }
      if (moved) swallowUntil = t + CLICK_SWALLOW_MS;
      if (wasMode === "swipe" || wasMode === "pan") {
        const wasAxis = lockAxis;
        const shift = dragX;
        const pan = dragY;
        dragX = dragY = 0;
        if (wasAxis === "h" && shift !== 0) {
          const dir = pageTurn(shift, v.vx);
          // Past the last image (or a no-turn release) the strip snaps back.
          if (dir !== 0 && host.step(dir)) tf = { scale: 1, x: 0, y: 0 };
        } else if (wasMode === "swipe" && wasAxis === "v" && dragClosesViewer(pan, v.vy)) {
          apply();
          host.close();
          return;
        }
        apply();
      }
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
      clearTapTimer();
      apply();
    },
    transform: () => tf,
    mode: () => mode,
  };
}

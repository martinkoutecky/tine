/** Left-edge swipe (GH #501, #492): iOS Back, and the swipe that opens the left
 * drawer (Logseq OG `components/container.cljs`, the closed sidebar's 10px
 * left-edge strip: pending at |dx| > 20, opens at dx > 40, no vertical check).
 *
 * ONE recognizer owns the left edge, so the two gestures cannot fight:
 *
 *   - The zone is the first EDGE_PX of the viewport on both touch platforms.
 *   - It is decided ONCE, at touchstart, by what that swipe would do:
 *       iOS, Back has something to pop (a transient layer, an open drawer, or
 *       a router entry)  -> BACK: the page follows the finger and the swipe
 *                           commits on release past a threshold, else snaps back.
 *       otherwise, when the left drawer exists (drawer mode) -> DRAWER: opens as
 *                           soon as the finger has travelled DRAWER_OPEN_PX.
 *       otherwise        -> nothing.
 *     Android always takes the DRAWER branch: its Back is the system gesture
 *     (src/androidBack.ts), which the OS takes from the same screen edge before
 *     the WebView sees it; a touch that reaches us started just inside it.
 *   - Unlike OG, a vertical-dominant start is never a swipe: the axis is locked
 *     within the first DECIDE_PX of travel and a vertical scroll is left alone.
 *
 * The Back sequence itself (transient, then drawer, then router history) is
 * src/appBack.ts and is never re-encoded here.
 *
 * Only a device can prove: that WKWebView / Android WebView deliver these touch
 * events from the physical screen edge at all (the OS gesture region), and the
 * feel of the finger-following animation. */

export const EDGE_PX = 24;
/** Travel before the axis is decided (px). */
export const DECIDE_PX = 8;
/** OG: the drawer opens once the finger is more than this far right. */
export const DRAWER_OPEN_PX = 40;
/** Back commits past this fraction of the viewport width, with a floor. */
export const BACK_COMMIT_FRACTION = 0.3;
export const BACK_COMMIT_MIN_PX = 64;
/** ...or on a flick: fast and at least this far. */
export const BACK_FLICK_MIN_PX = 32;
export const BACK_FLICK_VELOCITY = 0.5; // px per ms

export type EdgeSwipeKind = "back" | "drawer";
export type EdgeSwipeState = "idle" | "undecided" | "tracking" | "ignored" | "done";

export interface EdgeSwipeHost {
  /** Would Back do something right now? (src/appBack.ts `appBackAvailable`). */
  backAvailable(): boolean;
  /** Is the left drawer there to open (mobile drawer mode, not already open)? */
  drawerOpenable(): boolean;
  /** iOS owns Back by edge swipe; Android's is the OS gesture. */
  platform: "ios" | "android";
  viewportWidth(): number;
  /** Back mode: the finger moved; the page should be translated by `dx`. */
  progress(dx: number): void;
  /** Back mode: the gesture ended. `committed` tells whether to perform Back. */
  settle(committed: boolean, dx: number): void;
  openDrawer(): void;
  back(): void;
}

export interface EdgeSwipe {
  start(x: number, y: number, t: number): boolean;
  move(x: number, y: number, t: number): "claim" | "pass";
  end(x: number, y: number, t: number): void;
  cancel(): void;
  readonly state: EdgeSwipeState;
  readonly kind: EdgeSwipeKind | null;
}

export function createEdgeSwipe(host: EdgeSwipeHost): EdgeSwipe {
  let state: EdgeSwipeState = "idle";
  let kind: EdgeSwipeKind | null = null;
  let x0 = 0;
  let y0 = 0;
  let t0 = 0;
  let lastDx = 0;

  const reset = () => {
    state = "idle";
    kind = null;
  };

  return {
    get state() { return state; },
    get kind() { return kind; },

    start(x, y, t) {
      reset();
      if (x < 0 || x > EDGE_PX) return false;
      kind = host.platform === "ios" && host.backAvailable()
        ? "back"
        : host.drawerOpenable() ? "drawer" : null;
      if (!kind) return false;
      x0 = x; y0 = y; t0 = t; lastDx = 0;
      state = "undecided";
      return true;
    },

    move(x, y, t) {
      if (state === "idle" || state === "ignored" || state === "done") return "pass";
      const dx = x - x0;
      const dy = y - y0;
      if (state === "undecided") {
        if (Math.abs(dx) < DECIDE_PX && Math.abs(dy) < DECIDE_PX) return "pass";
        // Axis lock: rightward and horizontal-dominant, or this is not ours.
        if (dx > 0 && dx > Math.abs(dy)) state = "tracking";
        else { state = "ignored"; kind = null; return "pass"; }
      }
      void t;
      lastDx = Math.max(0, dx);
      if (kind === "drawer") {
        if (dx > DRAWER_OPEN_PX) {
          state = "done";
          host.openDrawer();
        }
      } else {
        host.progress(lastDx);
      }
      return "claim";
    },

    end(x, _y, t) {
      if (state !== "tracking") { reset(); return; }
      const dx = Math.max(0, x - x0);
      const k = kind;
      reset();
      if (k !== "back") return;
      const elapsed = Math.max(1, t - t0);
      const width = host.viewportWidth() || 1;
      const far = dx >= Math.max(BACK_COMMIT_MIN_PX, width * BACK_COMMIT_FRACTION);
      const flick = dx >= BACK_FLICK_MIN_PX && dx / elapsed >= BACK_FLICK_VELOCITY;
      const committed = far || flick;
      host.settle(committed, dx);
      if (committed) host.back();
    },

    cancel() {
      if (state === "tracking" && kind === "back") host.settle(false, lastDx);
      reset();
    },
  };
}

/** Elements that own their own horizontal pan (a code block, a table, the image
 * viewer) opt out; a swipe starting inside one is theirs. */
export const EDGE_SWIPE_IGNORE_SELECTOR = "[data-edge-swipe-ignore]";

export interface EdgeSwipeInstallDeps {
  platform: "ios" | "android" | "desktop";
  backAvailable(): boolean;
  drawerOpenable(): boolean;
  back(): void;
  openDrawer(): void;
  /** The element translated while Back follows the finger. */
  surface(): HTMLElement | null;
  doc?: Document;
  viewportWidth?: () => number;
}

/** Install the recognizer on touch platforms; desktop installs nothing.
 * Returns an idempotent cleanup. All work per touch event is O(1). */
export function installEdgeSwipe(deps: EdgeSwipeInstallDeps): () => void {
  if (deps.platform === "desktop") return () => {};
  const doc = deps.doc ?? document;
  const platform = deps.platform;
  const viewportWidth = deps.viewportWidth ?? (() => window.innerWidth);
  let snapTimer: ReturnType<typeof setTimeout> | null = null;

  const clearSurface = (surface: HTMLElement | null) => {
    if (!surface) return;
    surface.style.transform = "";
    surface.style.transition = "";
    surface.style.willChange = "";
  };

  const swipe = createEdgeSwipe({
    platform,
    backAvailable: deps.backAvailable,
    drawerOpenable: deps.drawerOpenable,
    viewportWidth,
    openDrawer: deps.openDrawer,
    back: deps.back,
    progress(dx) {
      const surface = deps.surface();
      if (!surface) return;
      if (snapTimer) { clearTimeout(snapTimer); snapTimer = null; }
      surface.style.transition = "none";
      surface.style.willChange = "transform";
      surface.style.transform = `translateX(${Math.round(dx)}px)`;
    },
    settle(committed, dx) {
      const surface = deps.surface();
      if (!surface) return;
      // Snap back: ease the page home. Commit: Back (called right after this)
      // swaps the page; the new page eases in from where the finger left it.
      surface.style.transition = "transform 160ms ease-out";
      surface.style.transform = committed ? `translateX(${Math.round(dx)}px)` : "translateX(0px)";
      if (committed) requestAnimationFrame(() => { surface.style.transform = "translateX(0px)"; });
      snapTimer = setTimeout(() => { snapTimer = null; clearSurface(surface); }, 200);
    },
  });

  const onStart = (event: TouchEvent) => {
    if (event.touches.length !== 1) { swipe.cancel(); return; }
    const target = event.target as Element | null;
    if (target?.closest?.(EDGE_SWIPE_IGNORE_SELECTOR)) return;
    const t = event.touches[0];
    swipe.start(t.clientX, t.clientY, event.timeStamp);
  };
  const onMove = (event: TouchEvent) => {
    if (event.touches.length !== 1) { swipe.cancel(); return; }
    const t = event.touches[0];
    if (swipe.move(t.clientX, t.clientY, event.timeStamp) === "claim" && event.cancelable) {
      // Horizontal edge drag: keep the OS from also treating it as a scroll or
      // as its own back/forward gesture.
      event.preventDefault();
    }
  };
  const onEnd = (event: TouchEvent) => {
    const t = event.changedTouches[0];
    if (t) swipe.end(t.clientX, t.clientY, event.timeStamp);
    else swipe.cancel();
  };
  const onCancel = () => swipe.cancel();

  doc.addEventListener("touchstart", onStart, { capture: true, passive: true });
  doc.addEventListener("touchmove", onMove, { capture: true, passive: false });
  doc.addEventListener("touchend", onEnd, { capture: true, passive: true });
  doc.addEventListener("touchcancel", onCancel, { capture: true, passive: true });
  let disposed = false;
  return () => {
    if (disposed) return;
    disposed = true;
    doc.removeEventListener("touchstart", onStart, { capture: true });
    doc.removeEventListener("touchmove", onMove, { capture: true });
    doc.removeEventListener("touchend", onEnd, { capture: true });
    doc.removeEventListener("touchcancel", onCancel, { capture: true });
    if (snapTimer) clearTimeout(snapTimer);
    clearSurface(deps.surface());
  };
}

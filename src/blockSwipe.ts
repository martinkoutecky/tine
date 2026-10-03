/** Horizontal swipes on a block row (GH #501): swipe right indents, a short
 * swipe left outdents, a long swipe left reveals the block's action menu.
 *
 * Modelled on Logseq OG mobile (`frontend/handler/block.cljs` on-touch-start /
 * on-touch-move / on-touch-end, attached to `.block-main-container` in
 * `components/block.cljs`). Kept from OG, because they are the user-visible
 * contract:
 *   - recognised once the finger is more than 30px sideways and less than 30px
 *     vertically from the origin (`SWIPE_RECOGNIZE_PX`, `VERTICAL_LIMIT_PX`);
 *   - indent at >= 40px right (`INDENT_PX`), outdent at 40..79px left
 *     (`OUTDENT_PX`), the action menu at >= 80px left (`ACTIONS_PX`);
 *   - nothing happens when the release travelled <= 10px (`RELEASE_MIN_PX`);
 *   - the direction is fixed by the first move; pulling the finger back
 *     re-bases the origin at that point (so pulling back disarms);
 *   - a range text selection, a second finger, or a touch that starts in a
 *     disabled zone (queries, property drawers, drawings, code blocks, media,
 *     the editor's own text area ...) is not a swipe;
 *   - while a block is being edited a swipe is only honoured within
 *     `EDITING_WINDOW_MS` of the touch start.
 *
 * Deliberately NOT OG:
 *   - OG decides scroll-versus-swipe only by the 30px vertical tolerance, so a
 *     sloppy vertical scroll can indent. Here the axis is locked within the
 *     first `AXIS_LOCK_PX` of travel: a vertical-dominant start is a scroll for
 *     the rest of the touch and is never a swipe (paired with
 *     `touch-action: pan-y` on the rows, src/styles/touchGestures.css).
 *   - OG's widths on a reversed finger are discontinuous (a left swipe pulled
 *     back to the right jumps to an 80px menu); here the travel along the fixed
 *     direction is simply clamped at zero.
 *   - OG resets the origin on any backward step; real fingers jitter, so a
 *     retreat only counts past `REVERSAL_PX`.
 *
 * Only a device can prove the feel (finger tracking, scroll-vs-swipe
 * arbitration by the OS compositor, haptics - OG adds a light haptic tap). */

export const SWIPE_RECOGNIZE_PX = 30;
export const VERTICAL_LIMIT_PX = 30;
export const INDENT_PX = 40;
export const OUTDENT_PX = 40;
export const ACTIONS_PX = 80;
export const RELEASE_MIN_PX = 10;
export const EDITING_WINDOW_MS = 600;
/** Travel before the axis is decided (px). */
export const AXIS_LOCK_PX = 6;
/** A retreat from the furthest point reached this far re-bases the origin. */
export const REVERSAL_PX = 4;

export type BlockSwipeAction = "indent" | "outdent" | "actions";
export type BlockSwipeState = "idle" | "undecided" | "tracking" | "ignored";

export interface BlockSwipeHost {
  /** Is the row's block being edited? (the 600ms window applies) */
  editing(): boolean;
  /** A range text selection exists (OG: selection type "Range"). */
  rangeSelected(): boolean;
  /** Feedback while tracking: the action a release now would run (null: none
   *  yet), and whether the swipe is recognised at all (past 30px). */
  progress(p: { action: BlockSwipeAction | null; recognized: boolean; dx: number }): void;
  /** The release ran an action. */
  commit(action: BlockSwipeAction, x: number, y: number): void;
}

export interface BlockSwipe {
  /** `touches` is the number of fingers down; only exactly one starts a swipe.
   *  `disabled` is the touchstart's disabled-zone verdict. */
  start(x: number, y: number, t: number, touches: number, disabled: boolean): boolean;
  /** Returns true when the swipe has claimed the touch (caller preventDefaults). */
  move(x: number, y: number, t: number, touches: number): boolean;
  end(x: number, y: number, t: number): void;
  cancel(): void;
  state(): BlockSwipeState;
}

/** The action a release at horizontal travel `travel` (signed, along the fixed
 *  direction already folded in: right > 0) would run. Pure; the thresholds'
 *  just-under / just-over tests call this directly. */
export function actionFor(direction: 1 | -1, travel: number): BlockSwipeAction | null {
  const along = Math.max(0, travel * direction);
  if (direction === 1) return along >= INDENT_PX ? "indent" : null;
  if (along >= ACTIONS_PX) return "actions";
  return along >= OUTDENT_PX ? "outdent" : null;
}

export function createBlockSwipe(host: BlockSwipeHost): BlockSwipe {
  let state: BlockSwipeState = "idle";
  let startT = 0;
  let x0 = 0;
  let y0 = 0;
  let dir: 1 | -1 | 0 = 0;
  let extreme = 0; // furthest x along `dir` since the last origin

  const reset = () => {
    if (state === "tracking") host.progress({ action: null, recognized: false, dx: 0 });
    state = "idle";
    dir = 0;
  };
  const ignore = () => {
    if (state === "tracking") host.progress({ action: null, recognized: false, dx: 0 });
    state = "ignored";
  };

  return {
    state: () => state,
    start(x, y, t, touches, disabled) {
      reset();
      if (touches !== 1 || disabled || host.rangeSelected()) {
        state = "ignored";
        return false;
      }
      state = "undecided";
      startT = t;
      x0 = x;
      y0 = y;
      extreme = x;
      return true;
    },
    move(x, y, t, touches) {
      if (state === "idle" || state === "ignored") return false;
      if (touches !== 1 || host.rangeSelected()) {
        ignore();
        return false;
      }
      if (host.editing() && t - startT >= EDITING_WINDOW_MS) {
        ignore();
        return false;
      }
      if (state === "undecided") {
        const dx = x - x0;
        const dy = y - y0;
        if (Math.max(Math.abs(dx), Math.abs(dy)) < AXIS_LOCK_PX) return false;
        // The axis is decided once. Vertical-dominant: a scroll, never a swipe.
        if (Math.abs(dx) <= Math.abs(dy)) {
          state = "ignored";
          return false;
        }
        state = "tracking";
        dir = dx > 0 ? 1 : -1;
        extreme = x;
      }
      // tracking
      if (dir === 1 ? x > extreme : x < extreme) extreme = x;
      // Retreat from the furthest point reached: re-base the origin here.
      if (dir * (extreme - x) > REVERSAL_PX) {
        x0 = x;
        y0 = y;
        extreme = x;
      }
      const dx = x - x0;
      const dy = y - y0;
      if (Math.abs(dy) >= VERTICAL_LIMIT_PX) {
        ignore();
        return false;
      }
      const recognized = Math.abs(dx) > SWIPE_RECOGNIZE_PX;
      host.progress({ action: recognized ? actionFor(dir as 1 | -1, dx) : null, recognized, dx });
      return true;
    },
    end(x, y, _t) {
      const wasTracking = state === "tracking";
      const d = dir;
      const travel = x - x0;
      const vertical = Math.abs(y - y0);
      reset();
      if (!wasTracking || d === 0) return;
      if (vertical >= VERTICAL_LIMIT_PX) return;
      if (Math.abs(travel) <= RELEASE_MIN_PX) return;
      const action = actionFor(d, travel);
      if (action) host.commit(action, x, y);
    },
    cancel() {
      reset();
    },
  };
}

/** Where a swipe must never start (OG `target-disable-swipe?` plus Tine's own
 *  equivalents). `.query-block`/`.query-result-sections` are OG's `.dsl-query`;
 *  `.block-properties` is OG's `.drawer`; `.drawio`/`.draw-wrap` its drawings;
 *  `pre.code-block` its code; the audio/video/iframe/pdf shields; and any
 *  element that itself scrolls sideways (tables, wide code), whose own pan must
 *  win. The editor's text area is excluded because its touches are caret and
 *  selection gestures. */
export const BLOCK_SWIPE_DISABLED_SELECTOR = [
  "textarea",
  "input",
  "select",
  "[contenteditable='true']",
  ".query-block",
  ".query-result-sections",
  ".block-properties",
  ".drawio",
  ".draw-wrap",
  "pre.code-block",
  ".calc",
  ".sheet-grid",
  ".sheet-scroll",
  ".md-table-wrap",
  ".sheet-board-wrap",
  ".audio-panel",
  ".audio-overlay",
  "audio",
  "video",
  "iframe",
  ".embed-iframe-wrap",
  ".pdf-viewer",
  "[data-block-swipe-ignore]",
].join(",");

function scrollsSideways(el: Element): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el.scrollWidth <= el.clientWidth + 1) return false;
  const overflowX = el.ownerDocument.defaultView?.getComputedStyle(el).overflowX;
  return overflowX === "auto" || overflowX === "scroll";
}

/** Is `target` (a touchstart target inside `row`) in a disabled zone? */
export function blockSwipeDisabledTarget(target: EventTarget | null, row: Element): boolean {
  const element = target instanceof Element ? target : null;
  if (!element) return true;
  // A nested row (embedded / referenced block) owns its own touches.
  if (element.closest(".block-main") !== row) return true;
  if (element.closest(BLOCK_SWIPE_DISABLED_SELECTOR)) return true;
  for (let el: Element | null = element; el && el !== row; el = el.parentElement) {
    if (scrollsSideways(el)) return true;
  }
  return false;
}

export interface BlockSwipeDeps {
  /** Which touch platform this is (null: swipes are not installed). */
  platform: "ios" | "android" | null;
  editing(): boolean;
  /** Run the action. `x`/`y` are the release point (the menu's anchor). */
  run(action: BlockSwipeAction, x: number, y: number): void;
}

/** Attach the swipe to a block row. Returns the cleanup. The listeners are
 *  passive except touchmove, which claims the touch (preventDefault) only once
 *  the horizontal axis is locked; `touch-action: pan-y` keeps vertical scroll
 *  native. A completed swipe also swallows the click that follows it and any
 *  native context menu while tracking, so it cannot arm the long-press menu. */
export function attachBlockSwipe(row: HTMLElement, deps: BlockSwipeDeps): () => void {
  if (!deps.platform) return () => {};
  const doc = row.ownerDocument;
  let committedAt = -Infinity;
  const swipe = createBlockSwipe({
    editing: deps.editing,
    rangeSelected: () => doc.getSelection()?.type === "Range",
    progress: ({ action, recognized, dx }) => {
      if (!recognized) {
        row.removeAttribute("data-swipe");
        row.style.removeProperty("--swipe-dx");
        return;
      }
      row.setAttribute("data-swipe", action ?? "pending");
      row.style.setProperty("--swipe-dx", `${Math.round(dx)}px`);
    },
    commit: (action, x, y) => {
      committedAt = Date.now();
      deps.run(action, x, y);
    },
  });
  const point = (e: TouchEvent): { x: number; y: number } | null => {
    const t = e.changedTouches[0] ?? e.touches[0];
    return t ? { x: t.clientX, y: t.clientY } : null;
  };
  const onStart = (e: TouchEvent) => {
    const p = point(e);
    if (!p) return;
    swipe.start(p.x, p.y, e.timeStamp, e.touches.length, blockSwipeDisabledTarget(e.target, row));
  };
  const onMove = (e: TouchEvent) => {
    const p = point(e);
    if (!p) return;
    if (swipe.move(p.x, p.y, e.timeStamp, e.touches.length) && e.cancelable) e.preventDefault();
  };
  const onEnd = (e: TouchEvent) => {
    const p = point(e);
    if (p && e.touches.length === 0) swipe.end(p.x, p.y, e.timeStamp);
    else swipe.cancel();
  };
  const onCancel = () => swipe.cancel();
  const onContextMenu = (e: Event) => {
    if (swipe.state() === "tracking") e.preventDefault();
  };
  const onClick = (e: Event) => {
    if (Date.now() - committedAt < 400) {
      e.preventDefault();
      e.stopPropagation();
      committedAt = -Infinity;
    }
  };
  row.addEventListener("touchstart", onStart, { passive: true });
  row.addEventListener("touchmove", onMove, { passive: false });
  row.addEventListener("touchend", onEnd, { passive: true });
  row.addEventListener("touchcancel", onCancel, { passive: true });
  row.addEventListener("contextmenu", onContextMenu, true);
  row.addEventListener("click", onClick, true);
  return () => {
    row.removeEventListener("touchstart", onStart);
    row.removeEventListener("touchmove", onMove);
    row.removeEventListener("touchend", onEnd);
    row.removeEventListener("touchcancel", onCancel);
    row.removeEventListener("contextmenu", onContextMenu, true);
    row.removeEventListener("click", onClick, true);
    row.removeAttribute("data-swipe");
    row.style.removeProperty("--swipe-dx");
  };
}

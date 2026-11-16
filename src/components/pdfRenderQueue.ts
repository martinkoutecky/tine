import { isMobilePlatform } from "../nativeChrome";

/** Pages rendered ahead of the viewport in the scroll direction. pdf.js renders
 * one; desktop takes two because a fit-width page is ~1.3 viewports tall and a
 * fast scroll otherwise outruns the single prefetch. Memory: one extra page
 * canvas each (~4.6 Mpx at fit-width DPR 1), inside PDF_CANVAS_CACHE_PIXEL_BUDGET,
 * and a prefetch never evicts a visible or other prefetched page. */
export const PDF_PREFETCH_PAGES = isMobilePlatform ? 1 : 2;

/** Quiet time after the last zoom input before visible pages re-render. The
 * stretched old raster stays on screen meanwhile (double-buffered), so this only
 * trades wasted mid-gesture renders against time-to-sharp. pdf.js uses 400 ms. */
export const PDF_ZOOM_SETTLE_MS = 200;

/** Ctrl+wheel pixel deltas per zoom step (pdf.js PIXELS_PER_LINE_SCALE). */
const PIXELS_PER_ZOOM_STEP = 30;

/** The next page to rasterize, after pdf.js's PDFRenderingQueue.getHighestPriority
 * (web/pdf_rendering_queue.js:106-165, v4.10.38): the visible pages in priority
 * order, then holes between them, then up to `prefetch` pages ahead in the
 * scroll direction (the other direction at the document's end). Null when
 * nothing needs work. O(visible + prefetch). */
export function highestPriorityPage(options: {
  visible: readonly number[];
  numPages: number;
  scrollingDown: boolean;
  prefetch: number;
  done: (page: number) => boolean;
}): number | null {
  const { visible, numPages, prefetch, done } = options;
  if (!visible.length) return null;
  for (const page of visible) if (!done(page)) return page;
  let first = visible[0], last = visible[0];
  for (const page of visible) {
    if (page < first) first = page;
    if (page > last) last = page;
  }
  for (let page = first + 1; page < last; page++) if (!done(page)) return page;
  const down = first <= 1 ? true : last >= numPages ? false : options.scrollingDown;
  for (let step = 1; step <= prefetch; step++) {
    const page = down ? last + step : first - step;
    if (page < 1 || page > numPages) break;
    if (!done(page)) return page;
  }
  return null;
}

/** Ctrl+wheel and trackpad-pinch accumulation, after pdf.js app.js
 * (#accumulateTicks / #accumulateFactor and the wheel handler, v4.10.38
 * web/app.js:2196-2222, 2562-2658): a wheel notch is one step; pixel deltas
 * count 30 px per step; a direction reversal drops the remainder; a pinch is a
 * continuous factor exp(-deltaY/100). */
export function createWheelZoomAccumulator() {
  let unusedTicks = 0;
  let unusedFactor = 1;

  function accumulateTicks(ticks: number): number {
    if ((unusedTicks > 0 && ticks < 0) || (unusedTicks < 0 && ticks > 0)) unusedTicks = 0;
    unusedTicks += ticks;
    // The epsilon keeps ten 0.1-step deltas from summing to 0.999… and losing a step.
    const whole = Math.trunc(unusedTicks + Math.sign(unusedTicks) * 1e-9) || 0;
    unusedTicks -= whole;
    return whole;
  }

  return {
    /** True when this Ctrl+wheel event is a trackpad pinch rather than a wheel. */
    isPinch(event: Pick<WheelEvent, "ctrlKey" | "deltaMode" | "deltaX" | "deltaY" | "deltaZ">, ctrlKeyDown: boolean) {
      return event.ctrlKey && !ctrlKeyDown && event.deltaMode === 0 && event.deltaX === 0
        && event.deltaZ === 0 && Math.abs(Math.exp(-event.deltaY / 100) - 1) < 0.05;
    },
    /** Whole zoom steps for a wheel event: positive zooms in. */
    steps(event: Pick<WheelEvent, "deltaMode" | "deltaX" | "deltaY">): number {
      let delta = Math.hypot(event.deltaX, event.deltaY);
      const angle = Math.atan2(event.deltaY, event.deltaX);
      if (-0.25 * Math.PI < angle && angle < 0.75 * Math.PI) delta = -delta;
      if (event.deltaMode !== 0) return Math.abs(delta) >= 1 ? Math.sign(delta) : accumulateTicks(delta);
      return accumulateTicks(delta / PIXELS_PER_ZOOM_STEP);
    },
    /** The factor to apply for a pinch event so rounding the scale to 1/100
     * never swallows a slow pinch. */
    pinchFactor(currentScale: number, deltaY: number): number {
      const factor = Math.exp(-deltaY / 100);
      if (factor === 1) return 1;
      if ((unusedFactor > 1 && factor < 1) || (unusedFactor < 1 && factor > 1)) unusedFactor = 1;
      const applied = Math.floor(currentScale * factor * unusedFactor * 100) / (100 * currentScale);
      unusedFactor = applied > 0 ? (factor * unusedFactor) / applied : 1;
      return applied;
    },
    reset() {
      unusedTicks = 0;
      unusedFactor = 1;
    },
  };
}

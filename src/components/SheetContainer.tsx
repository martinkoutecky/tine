import { tableBleedGeometry } from "./TableWrap";
import {
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";
import { SheetContainerOverlayContext } from "./SheetContainerOverlay";
import { newResizeObserver, requestFrame, viewportOf, windowOf } from "../windowRealm";

function px(value: string): number {
  const n = Number.parseFloat(value);
  return Number.isFinite(n) ? n : 0;
}

/** A sheet viewport. Resets horizontal scroll when its view changes and bleeds
 * wider content within its own pane by default; nested cells stay contained.
 * Measurement work is O(1), local to the mounted sheet, with no graph reads. */
export function SheetContainer(props: { children: JSX.Element; allowBreakout?: boolean }): JSX.Element {
  let el: HTMLDivElement | undefined;
  let scrollEl: HTMLDivElement | undefined;
  // Frames run on the container's own window clock (OG-MULTIWINDOW P1).
  let frame: (() => void) | undefined;
  let verifyFrame: (() => void) | undefined;
  let verifyBudget = 12;
  const settleFrames = new Set<() => void>();
  const settleTimers = new Set<number>();
  const [hovering, setHovering] = createSignal(false);
  const [corner, setCornerSignal] = createSignal<JSX.Element | null>(null);
  const overlay = {
    hovering,
    setCorner(node: JSX.Element | null) {
      setCornerSignal(() => node);
    },
  };

  const measure = () => {
    if (!el) return;
    frame = undefined;
    const nested = !!el.closest(".sheet-cell");
    const surface = scrollEl?.firstElementChild as HTMLElement | null;
    const style = getComputedStyle(el);
    // The BASE indent, not the effective margin: with .sheet-breakout applied,
    // computed margin-left already contains the previous shift, and deriving the
    // next shift from it oscillates (shift_new = shift_true - shift_old — the
    // parity-dependent off-center flake). The indent var is shift-free.
    const marginLeft = px(style.getPropertyValue("--sheet-container-indent")) || px(style.marginLeft);
    // The margin CURRENTLY in effect (shift included when .sheet-breakout is on)
    // — subtracting it from the element's own rect gives the parent content
    // edge regardless of parent padding (the macro path's parent has padding
    // that parentRect.left misses).
    const effMarginLeft = px(style.marginLeft);
    const marginRight = px(style.marginRight);
    const parentWidth = el.parentElement?.clientWidth ?? 0;
    const normalWidth = Math.max(0, parentWidth - marginLeft - marginRight) || el.clientWidth;
    const naturalWidth = Math.max(
      surface?.scrollWidth ?? 0,
      surface?.getBoundingClientRect().width ?? 0,
      surface ? 0 : scrollEl?.scrollWidth ?? el.scrollWidth
    );

    const main = el.closest(".main-content, .rs-item-body, .right-sidebar-body") as HTMLElement | null;
    if (main) {
      const mainRect = main.getBoundingClientRect();
      const viewport = viewportOf(el);
      const viewportRight = viewport.visual?.width ?? (viewport.width || el.ownerDocument.documentElement.clientWidth);
      const parentRight = main.parentElement?.getBoundingClientRect().right ?? 0;
      const stableRight = Math.min(
        mainRect.right,
        ...[viewportRight, parentRight].filter((right) => right > 0)
      );
      const transientRightOverflow = Math.max(0, mainRect.right - stableRight);
      const mainLeft = mainRect.left - transientRightOverflow;
      const normalLeft = el.getBoundingClientRect().left - effMarginLeft + marginLeft;
      const column = el.closest<HTMLElement>(".main-content-inner");
      const columnRect = column?.getBoundingClientRect();
      const { width: breakoutWidth, shift: breakoutShift } = tableBleedGeometry(
        normalLeft, normalWidth, naturalWidth, mainLeft, main.clientWidth || mainRect.width,
        columnRect ? (columnRect.left + columnRect.right) / 2 : mainLeft + mainRect.width / 2
      );
      el.style.setProperty("--sheet-breakout-width", `${Math.round(breakoutWidth)}px`);
      el.style.setProperty("--sheet-breakout-shift", `${breakoutShift}px`);
    }

    el.classList.toggle("sheet-breakout", props.allowBreakout !== false && !nested && (props.allowBreakout === true || !!surface?.matches(".sheet-table, .sheet-grid")) && naturalWidth > normalWidth + 1);
    scheduleVerify();
  };

  const scheduleVerify = () => {
    if (verifyFrame) return;
    verifyFrame = requestFrame(el, () => {
      verifyFrame = undefined;
      if (!el || !el.classList.contains("sheet-breakout")) return;
      const main = el.closest(".main-content, .rs-item-body, .right-sidebar-body") as HTMLElement | null;
      if (!main) return;
      const m = main.getBoundingClientRect();
      const r = el.getBoundingClientRect();
      const column = el.closest<HTMLElement>(".main-content-inner")?.getBoundingClientRect();
      const { shift } = tableBleedGeometry(r.left, r.width, r.width + 1, m.left, main.clientWidth || m.width,
        column ? (column.left + column.right) / 2 : (m.left + m.right) / 2);
      const expectedLeft = r.left - shift;
      if (Math.abs(r.left - expectedLeft) > 2 && verifyBudget > 0) {
        verifyBudget--;
        scheduleMeasureRaw();
      }
    });
  };

  const scheduleMeasureRaw = () => {
    if (frame) return;
    frame = requestFrame(el, () => {
      frame = requestFrame(el, measure);
    });
  };

  const scheduleMeasure = () => {
    verifyBudget = 12;
    scheduleMeasureRaw();
  };

  const scheduleMeasureAfterFrames = (frames: number) => {
    const step = (remaining: number) => {
      const id = requestFrame(el, () => {
        settleFrames.delete(id);
        if (remaining <= 1) scheduleMeasure();
        else step(remaining - 1);
      });
      settleFrames.add(id);
    };
    step(frames);
  };

  const scheduleMeasureAfterDelay = (ms: number) => {
    const id = window.setTimeout(() => {
      settleTimers.delete(id);
      verifyBudget = 12;
      measure();
    }, ms);
    settleTimers.add(id);
  };

  const cancelScheduledMeasures = () => {
    frame?.();
    frame = undefined;
    verifyFrame?.();
    verifyFrame = undefined;
    for (const cancel of settleFrames) cancel();
    settleFrames.clear();
    for (const id of settleTimers) window.clearTimeout(id);
    settleTimers.clear();
  };

  onMount(() => {
    if (!el) return;
    scheduleMeasureAfterFrames(2);
    scheduleMeasureAfterFrames(5);
    scheduleMeasureAfterDelay(150);
    scheduleMeasureAfterDelay(500);
    scheduleMeasureAfterDelay(750);
    const fonts = document.fonts;
    if (fonts?.ready) void fonts.ready.then(() => {
      scheduleMeasureAfterFrames(2);
      scheduleMeasureAfterDelay(150);
    }, () => scheduleMeasureAfterFrames(2));
    const main = el.closest<HTMLElement>(".main-content, .rs-item-body, .right-sidebar-body");
    let surface = scrollEl?.firstElementChild ?? null;
    let resizeObserver: ResizeObserver | null = null;
    const surfaceObserver = typeof MutationObserver === "undefined" || !scrollEl
      ? null
      : new MutationObserver(() => {
          const next = scrollEl?.firstElementChild ?? null;
          if (next === surface) return;
          if (surface) resizeObserver?.unobserve(surface);
          surface = next;
          if (surface) resizeObserver?.observe(surface);
          if (scrollEl) scrollEl.scrollLeft = 0;
          scheduleMeasure();
        });
    surfaceObserver?.observe(scrollEl!, { childList: true });
    onCleanup(() => surfaceObserver?.disconnect());
    const ownerWindow = windowOf(el);
    const realmObserver = newResizeObserver(el, scheduleMeasure);
    if (!realmObserver) {
      ownerWindow.addEventListener("resize", scheduleMeasure);
      onCleanup(() => {
        cancelScheduledMeasures();
        ownerWindow.removeEventListener("resize", scheduleMeasure);
      });
      return;
    }
    resizeObserver = realmObserver;
    resizeObserver.observe(el);
    if (main) resizeObserver.observe(main);
    if (scrollEl) resizeObserver.observe(scrollEl);
    if (surface) resizeObserver.observe(surface);
    if (el.parentElement) resizeObserver.observe(el.parentElement);
    ownerWindow.addEventListener("resize", scheduleMeasure);
    onCleanup(() => {
      cancelScheduledMeasures();
      resizeObserver?.disconnect();
      ownerWindow.removeEventListener("resize", scheduleMeasure);
    });
  });

  return (
    <SheetContainerOverlayContext.Provider value={overlay}>
      <div
        ref={(node) => {
          el = node;
        }}
        class="block-sheet-container"
        onPointerEnter={() => setHovering(true)}
        onPointerLeave={() => setHovering(false)}
      >
        <div
          ref={(node) => {
            scrollEl = node;
          }}
          class="sheet-scroll"
        >
          {props.children}
        </div>
        {corner()}
      </div>
    </SheetContainerOverlayContext.Provider>
  );
}

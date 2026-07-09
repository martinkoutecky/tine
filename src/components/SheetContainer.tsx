import {
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";
import { SheetContainerOverlayContext } from "./SheetContainerOverlay";

function px(value: string): number {
  const n = Number.parseFloat(value);
  return Number.isFinite(n) ? n : 0;
}

const sheetContainerMeasures = new Set<() => void>();
let mainContentObserver: ResizeObserver | null = null;
let observedMainContent: HTMLElement | null = null;

function scheduleAllSheetContainerMeasures(): void {
  for (const schedule of sheetContainerMeasures) schedule();
}

function observeMainContentForSheets(main: HTMLElement | null, schedule: () => void): () => void {
  sheetContainerMeasures.add(schedule);

  if (typeof ResizeObserver !== "undefined" && main) {
    if (observedMainContent !== main) {
      mainContentObserver?.disconnect();
      observedMainContent = main;
      mainContentObserver = new ResizeObserver(scheduleAllSheetContainerMeasures);
      mainContentObserver.observe(main);
    }
  }

  return () => {
    sheetContainerMeasures.delete(schedule);
    if (sheetContainerMeasures.size === 0) {
      mainContentObserver?.disconnect();
      mainContentObserver = null;
      observedMainContent = null;
    }
  };
}

export function SheetContainer(props: { children: JSX.Element }): JSX.Element {
  let el: HTMLDivElement | undefined;
  let scrollEl: HTMLDivElement | undefined;
  let frame = 0;
  let verifyFrame = 0;
  let verifyBudget = 12;
  const settleFrames = new Set<number>();
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
    frame = 0;
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

    const main = el.closest(".main-content") as HTMLElement | null;
    if (main) {
      const mainRect = main.getBoundingClientRect();
      const gutter = 20;
      const viewportRight = window.visualViewport?.width ?? (window.innerWidth || document.documentElement.clientWidth);
      const parentRight = main.parentElement?.getBoundingClientRect().right ?? 0;
      const stableRight = Math.min(
        mainRect.right,
        ...[viewportRight, parentRight].filter((right) => right > 0)
      );
      const transientRightOverflow = Math.max(0, mainRect.right - stableRight);
      const mainLeft = mainRect.left - transientRightOverflow;
      const fullSpan = Math.max(0, mainRect.width - gutter * 2);
      const normalLeft = el.getBoundingClientRect().left - effMarginLeft + marginLeft;
      const breakoutWidth = Math.max(normalWidth, fullSpan > 0 ? Math.min(naturalWidth, fullSpan) : naturalWidth);
      const spanLeft = mainLeft + gutter;
      const spanRight = mainLeft + mainRect.width - gutter;
      const centered = (spanLeft + spanRight) / 2 - breakoutWidth / 2;
      const breakoutLeft = Math.min(Math.max(centered, spanLeft), Math.max(spanLeft, spanRight - breakoutWidth));
      const breakoutShift = Math.round(normalLeft - breakoutLeft);
      el.style.setProperty("--sheet-breakout-width", `${Math.round(breakoutWidth)}px`);
      el.style.setProperty("--sheet-breakout-shift", `${breakoutShift}px`);
    }

    el.classList.toggle("sheet-breakout", !nested && naturalWidth > normalWidth + 1);
    scheduleVerify();
  };

  const scheduleVerify = () => {
    if (verifyFrame) return;
    verifyFrame = requestAnimationFrame(() => {
      verifyFrame = 0;
      if (!el || !el.classList.contains("sheet-breakout")) return;
      const main = el.closest(".main-content") as HTMLElement | null;
      if (!main) return;
      const m = main.getBoundingClientRect();
      const r = el.getBoundingClientRect();
      const gutter = 20;
      const spanLeft = m.left + gutter;
      const spanRight = m.right - gutter;
      const expectedLeft = Math.min(
        Math.max((spanLeft + spanRight) / 2 - r.width / 2, spanLeft),
        Math.max(spanLeft, spanRight - r.width)
      );
      if (Math.abs(r.left - expectedLeft) > 2 && verifyBudget > 0) {
        verifyBudget--;
        scheduleMeasureRaw();
      }
    });
  };

  const scheduleMeasureRaw = () => {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(measure);
    });
  };

  const scheduleMeasure = () => {
    verifyBudget = 12;
    scheduleMeasureRaw();
  };

  const scheduleMeasureAfterFrames = (frames: number) => {
    const step = (remaining: number) => {
      const id = requestAnimationFrame(() => {
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
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    if (verifyFrame) cancelAnimationFrame(verifyFrame);
    verifyFrame = 0;
    for (const id of settleFrames) cancelAnimationFrame(id);
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
    const unobserveMain = observeMainContentForSheets(el.closest(".main-content") as HTMLElement | null, scheduleMeasure);
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", scheduleMeasure);
      onCleanup(() => {
        cancelScheduledMeasures();
        window.removeEventListener("resize", scheduleMeasure);
        unobserveMain();
      });
      return;
    }
    const ro = new ResizeObserver(scheduleMeasure);
    ro.observe(el);
    if (scrollEl) ro.observe(scrollEl);
    if (scrollEl?.firstElementChild) ro.observe(scrollEl.firstElementChild);
    if (el.parentElement) ro.observe(el.parentElement);
    window.addEventListener("resize", scheduleMeasure);
    onCleanup(() => {
      cancelScheduledMeasures();
      ro.disconnect();
      window.removeEventListener("resize", scheduleMeasure);
      unobserveMain();
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

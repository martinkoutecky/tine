import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createLongPress, LONG_PRESS_DELAY, LONG_PRESS_MOVE_TOLERANCE } from "./longPress";
import { shouldOpenTextContextMenu } from "../contextMenuPolicy";

// GH #231: a deliberate long-press on a link must surface the same context
// menu desktop right-click gives. The recognizer dispatches a SYNTHETIC
// contextmenu at the held point, so it always goes through the same menu code
// path (which preventDefaults and therefore also suppresses the browser's
// long-press text-selection behavior).

function pointer(type: string, x: number, y: number, extra: Partial<PointerEventInit> = {}): PointerEvent {
  return new PointerEvent(type, {
    bubbles: true,
    cancelable: true,
    pointerType: "touch",
    isPrimary: true,
    clientX: x,
    clientY: y,
    ...extra,
  });
}

describe("createLongPress", () => {
  let el: HTMLAnchorElement;
  let handlers: ReturnType<typeof createLongPress>;
  let heard: MouseEvent[];

  beforeEach(() => {
    vi.useFakeTimers();
    el = document.createElement("a");
    document.body.appendChild(el);
    heard = [];
    el.addEventListener("contextmenu", (e) => heard.push(e));
    handlers = createLongPress(() => el);
    for (const [k, v] of Object.entries(handlers) as [string, ((e: PointerEvent) => void) | undefined][]) {
      if (k.startsWith("on")) el.addEventListener(k.slice(2).toLowerCase(), (e) => (v as (e: PointerEvent) => void)(e as PointerEvent));
    }
  });

  afterEach(() => {
    handlers.dispose();
    el.remove();
    vi.useRealTimers();
  });

  it("fires one contextmenu after a still hold through the delay", () => {
    el.dispatchEvent(pointer("pointerdown", 40, 60));
    vi.advanceTimersByTime(LONG_PRESS_DELAY - 1);
    expect(heard).toHaveLength(0);
    vi.advanceTimersByTime(1);
    expect(heard).toHaveLength(1);
    expect(heard[0].clientX).toBe(40);
    expect(heard[0].clientY).toBe(60);
    expect(shouldOpenTextContextMenu(heard[0], true)).toBe(true);
    expect(handlers.consumeClick()).toBe(true);
    // Exactly once: releasing late must not re-fire anything.
    el.dispatchEvent(pointer("pointerup", 40, 60));
    expect(handlers.consumeClick()).toBe(true);
    expect(handlers.consumeClick()).toBe(false);
    vi.advanceTimersByTime(2 * LONG_PRESS_DELAY);
    expect(heard).toHaveLength(1);
  });

  it("consumes the held pointer's retargeted overlay click while preserving keyboard and the next gesture", () => {
    const overlay = document.createElement("div");
    document.body.append(overlay);
    let clicks = 0;
    overlay.addEventListener("click", () => clicks++);
    try {
      el.dispatchEvent(pointer("pointerdown", 40, 60, { pointerId: 7 }));
      vi.advanceTimersByTime(LONG_PRESS_DELAY);
      el.dispatchEvent(pointer("pointerup", 40, 60, { pointerId: 7 }));
      overlay.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 0 }));
      expect(clicks).toBe(1);
      overlay.dispatchEvent(pointer("click", 40, 60, { pointerId: 7, detail: 1 }));
      expect(clicks).toBe(1);
      overlay.dispatchEvent(pointer("pointerdown", 40, 60, { pointerId: 8 }));
      overlay.dispatchEvent(pointer("click", 40, 60, { pointerId: 8, detail: 1 }));
      expect(clicks).toBe(2);
    } finally { overlay.remove(); }
  });

  it("consumes the same hold's native contextmenu before the overlay and still consumes its later click", () => {
    const overlay = document.createElement("div");
    document.body.append(overlay);
    let menus = 0, clicks = 0;
    overlay.addEventListener("contextmenu", () => menus++);
    overlay.addEventListener("click", () => clicks++);
    try {
      el.dispatchEvent(pointer("pointerdown", 40, 60, { pointerId: 7 }));
      vi.advanceTimersByTime(LONG_PRESS_DELAY);
      expect(heard).toHaveLength(1); // Tine's owned synthetic menu still arrives.
      overlay.dispatchEvent(pointer("contextmenu", 40, 66, { pointerId: 7 }));
      expect(menus).toBe(0);
      overlay.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
      expect(menus).toBe(1); // Keyboard context menu remains independent.
      el.dispatchEvent(pointer("pointerup", 40, 60, { pointerId: 7 }));
      overlay.dispatchEvent(pointer("click", 40, 60, { pointerId: 7, detail: 1 }));
      expect(clicks).toBe(0);
    } finally { overlay.remove(); }
  });

  it("does not consume another pointer and removes capture ownership on dispose", () => {
    const overlay = document.createElement("div");
    document.body.append(overlay);
    let clicks = 0;
    overlay.addEventListener("click", () => clicks++);
    try {
      el.dispatchEvent(pointer("pointerdown", 40, 60, { pointerId: 7 }));
      vi.advanceTimersByTime(LONG_PRESS_DELAY);
      overlay.dispatchEvent(pointer("click", 40, 60, { pointerId: 8, detail: 1 }));
      expect(clicks).toBe(1);
      handlers.dispose();
      overlay.dispatchEvent(pointer("click", 40, 60, { pointerId: 7, detail: 1 }));
      expect(clicks).toBe(2);
    } finally { overlay.remove(); }
  });

  it("releases a fired hold guard for a new primary gesture even if no compatibility click arrived", () => {
    el.dispatchEvent(pointer("pointerdown", 40, 60, { pointerId: 7 }));
    vi.advanceTimersByTime(LONG_PRESS_DELAY);
    el.dispatchEvent(pointer("pointerup", 40, 60, { pointerId: 7 }));
    el.dispatchEvent(pointer("pointerdown", 40, 60, { pointerId: 8 }));
    expect(handlers.consumeClick()).toBe(false);
  });

  it("cancels when the pointer moves past the tolerance before the delay", () => {
    el.dispatchEvent(pointer("pointerdown", 40, 60));
    vi.advanceTimersByTime(60);
    el.dispatchEvent(pointer("pointermove", 40 + LONG_PRESS_MOVE_TOLERANCE + 1, 60));
    vi.advanceTimersByTime(2 * LONG_PRESS_DELAY);
    expect(heard).toHaveLength(0);
    // Small jitter within tolerance does NOT cancel.
    el.dispatchEvent(pointer("pointerdown", 40, 60));
    el.dispatchEvent(pointer("pointermove", 40 + LONG_PRESS_MOVE_TOLERANCE - 1, 60 - LONG_PRESS_MOVE_TOLERANCE + 1));
    vi.advanceTimersByTime(LONG_PRESS_DELAY);
    expect(heard).toHaveLength(1);
  });

  it("cancels on quick tap release (tap stays plain navigation, no menu)", () => {
    el.dispatchEvent(pointer("pointerdown", 40, 60));
    el.dispatchEvent(pointer("pointerup", 40, 60));
    vi.advanceTimersByTime(2 * LONG_PRESS_DELAY);
    expect(heard).toHaveLength(0);
  });

  it("cancels on pointercancel (scroll interception) without firing", () => {
    el.dispatchEvent(pointer("pointerdown", 40, 60));
    el.dispatchEvent(pointer("pointercancel", 40, 60));
    vi.advanceTimersByTime(2 * LONG_PRESS_DELAY);
    expect(heard).toHaveLength(0);
  });

  it("ignores mouse and secondary pointers entirely (desktop uses right-click)", () => {
    el.dispatchEvent(pointer("pointerdown", 40, 60, { pointerType: "mouse" }));
    vi.advanceTimersByTime(2 * LONG_PRESS_DELAY);
    expect(heard).toHaveLength(0);
    el.dispatchEvent(pointer("pointerdown", 40, 60, { isPrimary: false }));
    vi.advanceTimersByTime(2 * LONG_PRESS_DELAY);
    expect(heard).toHaveLength(0);
    // A pen long-press DOES count (stylus has no right click either).
    el.dispatchEvent(pointer("pointerdown", 40, 60, { pointerType: "pen" }));
    vi.advanceTimersByTime(LONG_PRESS_DELAY);
    expect(heard).toHaveLength(1);
  });

  it("re-arms cleanly for the next gesture", () => {
    el.dispatchEvent(pointer("pointerdown", 40, 60));
    vi.advanceTimersByTime(LONG_PRESS_DELAY);
    el.dispatchEvent(pointer("pointerup", 40, 60));
    el.dispatchEvent(pointer("pointerdown", 100, 120));
    vi.advanceTimersByTime(LONG_PRESS_DELAY);
    expect(heard).toHaveLength(2);
    expect(heard[1].clientX).toBe(100);
    expect(heard[1].clientY).toBe(120);
  });
});

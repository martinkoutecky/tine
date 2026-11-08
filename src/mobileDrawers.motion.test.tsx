import { onCleanup, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MobileDrawerController, MobileDrawerPanel, dismissDrawerAndRestore } from "./components/MobileDrawerShell";
import { activeDrawer, setLeftSidebarOpen, setRightSidebarOpen, sidebarOpen, rightSidebarOpen } from "./ui";
import { dragOpeningMobileDrawer, settleMobileDrawer } from "./mobileDrawerMotion";

let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); dispose = undefined; vi.useRealTimers(); vi.restoreAllMocks(); document.body.replaceChildren(); document.documentElement.removeAttribute("dir"); });

function harness(phone = true, reduced = false) {
  vi.useFakeTimers();
  const listeners = new Set<() => void>();
  vi.stubGlobal("matchMedia", (query: string) => ({
    get matches() { return query.includes("reduced-motion") ? reduced : phone; },
    addEventListener: (_: string, fn: () => void) => listeners.add(fn),
    removeEventListener: (_: string, fn: () => void) => listeners.delete(fn),
  }));
  // Deterministic compositor clock: assertions cover frames/lifetime, while the
  // native journey samples real WebKit transforms and opacity between frames.
  const animations: { element: HTMLElement; frames: Keyframe[]; options: KeyframeAnimationOptions; cancel: ReturnType<typeof vi.fn> }[] = [];
  Object.defineProperty(Element.prototype, "animate", { configurable: true, writable: true, value: vi.fn(function(this: HTMLElement, frames: Keyframe[], options: KeyframeAnimationOptions) {
    const cancel = vi.fn();
    animations.push({ element: this, frames, options, cancel });
    return { cancel, onfinish: null } as unknown as Animation;
  }) });
  setLeftSidebarOpen(false); setRightSidebarOpen(false);
  let disposedChildren = 0;
  function Content(): JSX.Element { onCleanup(() => disposedChildren++); return <button class="navigate" onClick={() => dismissDrawerAndRestore("navigation")}>Page link</button>; }
  const root = document.createElement("div"); document.body.append(root);
  const unmount = render(() => <>
    <MobileDrawerPanel open={sidebarOpen()} side="left" label="Navigation" class="left-sidebar"><Content /></MobileDrawerPanel>
    <MobileDrawerPanel open={rightSidebarOpen()} side="right" label="References" class="right-sidebar"><Content /></MobileDrawerPanel>
    <main class="main-content" tabindex={-1} />
    <MobileDrawerController />
  </>, root);
  dispose = () => { unmount(); setLeftSidebarOpen(false); setRightSidebarOpen(false); vi.unstubAllGlobals(); delete (Element.prototype as Partial<Element>).animate; };
  return { root, animations, disposedChildren: () => disposedChildren, reduce() { reduced = true; listeners.forEach(fn => fn()); } };
}
const tick = async () => { await Promise.resolve(); await Promise.resolve(); };
function touch(el: HTMLElement, type: string, x: number, y = 100) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.assign(event, { touches: type === "touchend" ? [] : [{ clientX: x, clientY: y }], changedTouches: [{ clientX: x, clientY: y }] });
  el.dispatchEvent(event);
  return event;
}

describe("GH #501 phone drawer motion (OG 300ms ease slide / 400ms ease scrim)", () => {
  for (const side of ["left", "right"] as const) {
    it(`${side}: slides in and out, isolates the departing surface immediately, then releases its work`, async () => {
      const h = harness(); await tick();
      expect(h.root.querySelector(`[data-mobile-drawer-panel="${side}"]`)).toBeNull();
      (side === "left" ? setLeftSidebarOpen : setRightSidebarOpen)(true); await tick();
      const panel = h.root.querySelector<HTMLElement>(`[data-mobile-drawer-panel="${side}"]`)!;
      expect(panel.dataset.drawerMotion).toBe("open");
      const slide = h.animations.find(a => a.element === panel)!;
      expect(slide.frames[0].transform).toBe(`translateX(${side === "left" ? "-100%" : "100%"})`);
      expect(slide.frames.at(-1)?.transform).toBe("translateX(0px)");
      expect(slide.options).toMatchObject({ duration: 300, easing: "ease" });
      const fade = h.animations.find(a => a.element.hasAttribute("data-mobile-drawer-scrim"))!;
      expect(fade.options).toMatchObject({ duration: 400, easing: "ease" });
      vi.advanceTimersByTime(450); await tick();
      panel.querySelector<HTMLButtonElement>(".navigate")!.click(); await tick();
      // Intentional OG difference: navigation/focus never waits for slide-out.
      expect(activeDrawer()).toBeNull();
      expect(document.activeElement).toBe(h.root.querySelector("main"));
      expect(panel.dataset.drawerMotion).toBe("closing");
      expect(panel.inert || panel.hasAttribute("inert")).toBe(true);
      expect(panel.getAttribute("aria-hidden")).toBe("true");
      expect(panel.getAttribute("role")).toBeNull();
      expect(h.disposedChildren()).toBe(0);
      vi.advanceTimersByTime(450); await tick();
      expect(panel.isConnected).toBe(false);
      expect(h.disposedChildren()).toBe(1);
      expect(h.root.querySelector("[data-mobile-drawer-scrim]")).toBeNull();
    });
  }
  it("Reduce Motion skips both animations, including a preference changed during slide-out", async () => {
    const h = harness(true, true); await tick();
    setLeftSidebarOpen(true); await tick();
    expect(h.root.querySelector(".left-sidebar")).not.toBeNull();
    expect(h.animations).toHaveLength(0);
    dismissDrawerAndRestore("explicit"); await tick();
    expect(h.root.querySelector(".left-sidebar")).toBeNull();
    expect(h.root.querySelector("[data-mobile-drawer-scrim]")).toBeNull();
  });
  it("finishes an in-flight exit immediately when Reduce Motion turns on", async () => {
    const h = harness(); await tick(); setRightSidebarOpen(true); await tick();
    dismissDrawerAndRestore("scrim"); await tick();
    expect(h.root.querySelector(".right-sidebar")).not.toBeNull();
    h.reduce(); await tick();
    expect(h.root.querySelector(".right-sidebar")).toBeNull();
    expect(h.root.querySelector("[data-mobile-drawer-scrim]")).toBeNull();
  });
  it("desktop keeps its immediate mount/unmount behavior", async () => {
    const h = harness(false); await tick(); setLeftSidebarOpen(true); await tick();
    expect(h.root.querySelector(".left-sidebar")).not.toBeNull();
    setLeftSidebarOpen(false); await tick();
    expect(h.root.querySelector(".left-sidebar")).toBeNull();
    expect(h.animations).toHaveLength(0);
  });
  for (const side of ["left", "right"] as const) {
    it(`${side}: follows the closing finger, snaps back on a short/cancelled drag, and finishes a long drag`, async () => {
      const h = harness(); await tick();
      (side === "left" ? setLeftSidebarOpen : setRightSidebarOpen)(true); await tick();
      vi.advanceTimersByTime(450);
      const panel = h.root.querySelector<HTMLElement>(`.${side}-sidebar`)!;
      Object.defineProperty(panel, "offsetWidth", { value: 300 });
      const sign = side === "left" ? -1 : 1;
      touch(panel, "touchstart", 150);
      expect(touch(panel, "touchmove", 150 + sign * 40).defaultPrevented).toBe(true);
      expect(panel.style.transform).toBe(`translateX(${sign * 40}px)`);
      expect(h.root.querySelector<HTMLElement>("[data-mobile-drawer-scrim]")!.style.opacity).toBeCloseTo(1 - 40 / 300);
      touch(panel, "touchend", 150 + sign * 40);
      expect(activeDrawer()).toBe(side);
      expect(h.animations.filter(a => a.element === panel).at(-1)?.frames[0].transform).toBe(`translateX(${sign * 40}px)`);
      vi.advanceTimersByTime(450);
      touch(panel, "touchstart", 150); touch(panel, "touchmove", 150 + sign * 70);
      touch(panel, "touchcancel", 150 + sign * 70);
      expect(activeDrawer()).toBe(side);
      vi.advanceTimersByTime(450);
      touch(panel, "touchstart", 150); touch(panel, "touchmove", 150 + sign * 110);
      touch(panel, "touchmove", 150 + sign * 140);
      expect(panel.style.transform).toBe(`translateX(${sign * 140}px)`);
      touch(panel, "touchend", 150 + sign * 140);
      expect(activeDrawer()).toBeNull();
      expect(h.animations.filter(a => a.element === panel).at(-1)?.frames[0].transform).toBe(`translateX(${sign * 140}px)`);
      vi.advanceTimersByTime(450);
      expect(panel.isConnected).toBe(false);
    });
  }
  it("mirrors edges in RTL and follows an opening edge drag through release", async () => {
    document.documentElement.dir = "rtl";
    const h = harness(); await tick(); setLeftSidebarOpen(true); await tick();
    const panel = h.root.querySelector<HTMLElement>(".left-sidebar")!;
    expect(h.animations.find(a => a.element === panel)?.frames[0].transform).toBe("translateX(100%)");
    Object.defineProperty(panel, "offsetWidth", { value: 300 });
    dragOpeningMobileDrawer(80);
    expect(panel.style.transform).toBe("translateX(220px)");
    dragOpeningMobileDrawer(120);
    expect(panel.style.transform).toBe("translateX(180px)");
    settleMobileDrawer("left");
    expect(h.animations.filter(a => a.element === panel).at(-1)?.frames[0].transform).toBe("translateX(180px)");
    vi.advanceTimersByTime(450);
    expect(panel.style.transform).toBe("");
  });
  it("reopening during exit cancels obsolete removal, including a switch to the other side", async () => {
    const h = harness(); await tick(); setLeftSidebarOpen(true); await tick();
    dismissDrawerAndRestore("escape"); await tick();
    setLeftSidebarOpen(true); await tick(); vi.advanceTimersByTime(450);
    expect(h.root.querySelector(".left-sidebar")).not.toBeNull();
    expect(activeDrawer()).toBe("left");
    setRightSidebarOpen(true); await tick();
    expect(h.root.querySelector<HTMLElement>(".left-sidebar")!.inert).toBe(true);
    expect(activeDrawer()).toBe("right");
    vi.advanceTimersByTime(450);
    expect(h.root.querySelector(".left-sidebar")).toBeNull();
    expect(h.root.querySelector(".right-sidebar")).not.toBeNull();
  });
  it("samples a running transition before cancelling it when direction reverses", async () => {
    const h = harness(); await tick(); setLeftSidebarOpen(true); await tick();
    const panel = h.root.querySelector<HTMLElement>(".left-sidebar")!;
    const original = window.getComputedStyle.bind(window);
    let transform = "matrix(1, 0, 0, 1, -140, 0)";
    vi.spyOn(window, "getComputedStyle").mockImplementation((el) => {
      const style = original(el);
      return el === panel ? new Proxy(style, { get(target, key) { return key === "transform" ? transform : Reflect.get(target, key, target); } }) : style;
    });
    h.animations.find(a => a.element === panel)!.cancel.mockImplementationOnce(() => { transform = "none"; });
    dismissDrawerAndRestore("escape");
    expect(h.animations.filter(a => a.element === panel).at(-1)?.frames[0].transform).toBe("matrix(1, 0, 0, 1, -140, 0)");
  });
});

import { afterEach, describe, expect, it, vi } from "vitest";
import { installEdgeSwipe } from "./edgeSwipe";

function touch(type: string, x: number, y: number, target: EventTarget = document.body, ts = 0) {
  const t = { clientX: x, clientY: y, identifier: 1, target };
  const e = new Event(type, { bubbles: true, cancelable: true }) as any;
  const list = type === "touchend" || type === "touchcancel" ? [] : [t];
  Object.defineProperty(e, "touches", { value: list });
  Object.defineProperty(e, "changedTouches", { value: [t] });
  Object.defineProperty(e, "timeStamp", { value: ts });
  target.dispatchEvent(e);
  return e as Event;
}

afterEach(() => { document.body.innerHTML = ""; });

function setup(platform: "ios" | "android" | "desktop", over: Record<string, unknown> = {}) {
  const surface = document.createElement("div");
  document.body.appendChild(surface);
  const deps = {
    platform,
    backAvailable: vi.fn(() => true),
    drawerOpenable: vi.fn(() => true),
    back: vi.fn(),
    openDrawer: vi.fn(),
    surface: () => surface,
    viewportWidth: () => 400,
    ...over,
  };
  const uninstall = installEdgeSwipe(deps as never);
  return { surface, deps, uninstall };
}

describe("installEdgeSwipe on the DOM", () => {
  it("installs nothing on desktop", () => {
    const { deps, uninstall } = setup("desktop");
    touch("touchstart", 5, 100); touch("touchmove", 100, 100); touch("touchend", 200, 100);
    expect(deps.back).not.toHaveBeenCalled();
    uninstall();
  });

  it("iOS: page follows the finger, commits past the threshold and cleans up", () => {
    vi.useFakeTimers();
    const { surface, deps, uninstall } = setup("ios");
    touch("touchstart", 5, 100);
    const move = touch("touchmove", 5 + 60, 100, document.body, 100);
    expect(move.defaultPrevented).toBe(true);
    expect(surface.style.transform).toBe("translateX(60px)");
    touch("touchmove", 5 + 150, 100, document.body, 300);
    touch("touchend", 5 + 150, 100, document.body, 300);
    expect(deps.back).toHaveBeenCalledOnce();
    vi.advanceTimersByTime(300);
    expect(surface.style.transform).toBe("");
    uninstall();
    vi.useRealTimers();
  });

  it("iOS: releasing under the threshold snaps back without going back", () => {
    vi.useFakeTimers();
    const { surface, deps, uninstall } = setup("ios");
    touch("touchstart", 5, 100);
    touch("touchmove", 5 + 50, 100, document.body, 1000);
    touch("touchend", 5 + 50, 100, document.body, 1000);
    expect(deps.back).not.toHaveBeenCalled();
    expect(surface.style.transform).toBe("translateX(0px)");
    vi.advanceTimersByTime(300);
    expect(surface.style.transform).toBe("");
    uninstall();
    vi.useRealTimers();
  });

  it("a vertical scroll from the edge is left to the browser", () => {
    const { deps, uninstall } = setup("ios");
    touch("touchstart", 5, 100);
    const move = touch("touchmove", 8, 160);
    expect(move.defaultPrevented).toBe(false);
    touch("touchend", 8, 260);
    expect(deps.back).not.toHaveBeenCalled();
    uninstall();
  });

  it("a touch outside the edge strip is never ours", () => {
    const { deps, uninstall } = setup("ios");
    touch("touchstart", 100, 100);
    const move = touch("touchmove", 300, 100);
    expect(move.defaultPrevented).toBe(false);
    touch("touchend", 300, 100);
    expect(deps.back).not.toHaveBeenCalled();
    uninstall();
  });

  it("Android edge pull opens the drawer", () => {
    const { deps, uninstall } = setup("android");
    touch("touchstart", 5, 100); touch("touchmove", 70, 100);
    expect(deps.openDrawer).toHaveBeenCalledOnce();
    touch("touchend", 70, 100);
    expect(deps.back).not.toHaveBeenCalled();
    uninstall();
  });

  it("an element that owns its horizontal pan opts out", () => {
    const { deps, uninstall } = setup("ios");
    const owner = document.createElement("div");
    owner.setAttribute("data-edge-swipe-ignore", "");
    document.body.appendChild(owner);
    touch("touchstart", 5, 100, owner); touch("touchmove", 200, 100, owner); touch("touchend", 200, 100, owner);
    expect(deps.back).not.toHaveBeenCalled();
    uninstall();
  });

  it("a second finger cancels the swipe; cleanup is idempotent and removes listeners", () => {
    vi.useFakeTimers();
    const { surface, deps, uninstall } = setup("ios");
    touch("touchstart", 5, 100); touch("touchmove", 60, 100);
    const two = new Event("touchmove", { bubbles: true, cancelable: true }) as any;
    Object.defineProperty(two, "touches", { value: [{}, {}] });
    document.body.dispatchEvent(two);
    touch("touchend", 200, 100);
    expect(deps.back).not.toHaveBeenCalled();
    uninstall(); uninstall();
    expect(surface.style.transform).toBe("");
    touch("touchstart", 5, 100); touch("touchmove", 60, 100);
    expect(surface.style.transform).toBe("");
    vi.useRealTimers();
  });
});

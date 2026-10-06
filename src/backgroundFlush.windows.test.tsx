// Review F2 (OG-MULTIWINDOW): main's own pagehide (a reload or navigation)
// takes every window's rendering with it, so the background flush runs even
// while a workspace window is still visible. A workspace window's own
// pagehide does not force it (that window's disposal flushes).
import { afterEach, describe, expect, it, vi } from "vitest";
import { installBackgroundFlush } from "./backgroundFlush";
import { mainWindow, registerWindow } from "./windowRealm";

const cleanups: (() => void)[] = [];
afterEach(() => { while (cleanups.length) cleanups.pop()!(); });

function popupWindow(): Window {
  const frame = document.createElement("iframe");
  document.body.append(frame);
  cleanups.push(() => frame.remove());
  return frame.contentWindow as Window;
}

describe("background flush with workspace windows (review F2)", () => {
  it("main's pagehide flushes although a workspace window is visible", () => {
    const popup = popupWindow();
    cleanups.push(registerWindow("ws-test", popup));
    const endEdit = vi.fn();
    const flushAll = vi.fn(() => Promise.resolve(true));
    // A workspace window is visible, so no window is hidden.
    cleanups.push(installBackgroundFlush({ endEdit, flushAll, closeInFlight: () => false, isHidden: () => false, externalActivityHeld: () => false }));
    popup.dispatchEvent(new Event("pagehide"));
    mainWindow.document.dispatchEvent(new Event("visibilitychange"));
    expect(flushAll).not.toHaveBeenCalled();
    mainWindow.dispatchEvent(new Event("pagehide"));
    expect(endEdit).toHaveBeenCalledOnce();
    expect(flushAll).toHaveBeenCalledOnce();
  });
});

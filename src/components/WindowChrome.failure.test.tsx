import { beforeEach, expect, it, vi } from "vitest";
import { setToasts, toasts } from "../toasts";

const windowMock = vi.hoisted(() => ({
  isMaximized: vi.fn(async () => false),
  onResized: vi.fn(async (_callback?: () => void) => () => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => windowMock }));

const platform = vi.hoisted(() => ({ isMobilePlatform: false }));
vi.mock("../nativeChrome", () => platform);

beforeEach(() => {
  vi.clearAllMocks();
  windowMock.isMaximized.mockResolvedValue(false);
  platform.isMobilePlatform = false;
  setToasts([]);
});

import { installWindowChrome, maximized } from "./WindowChrome";

it("reports a native window-state read failure with fixed text", async () => {
  setToasts([]);
  windowMock.isMaximized.mockRejectedValueOnce(new Error("private window title"));
  const cleanup = installWindowChrome();
  try {
    await vi.waitFor(() => expect(toasts().some((toast) => toast.message === "Couldn't read the window state.")).toBe(true));
    expect(toasts().map((toast) => toast.message).join(" ")).not.toContain("private window title");
  } finally {
    cleanup();
  }
});

// GH #621: Tauri's desktop maximize API is unavailable on mobile. Start through
// the same installer App calls, with the native platform answer injected.
it.each(["android", "ios"])("starts on %s without reading desktop window state", (os) => {
  platform.isMobilePlatform = os === "android" || os === "ios";
  const cleanup = installWindowChrome();
  cleanup();
  expect(windowMock.isMaximized).not.toHaveBeenCalled();
  expect(windowMock.onResized).not.toHaveBeenCalled();
  expect(toasts()).toEqual([]);
});

it("tracks desktop maximize changes and releases the resize listener", async () => {
  let resize = () => {};
  const unlisten = vi.fn();
  windowMock.onResized.mockImplementationOnce(async (callback?: () => void) => {
    resize = callback!;
    return unlisten;
  });
  const cleanup = installWindowChrome();
  await vi.waitFor(() => expect(windowMock.onResized).toHaveBeenCalledOnce());
  windowMock.isMaximized.mockResolvedValueOnce(true);
  resize();
  await vi.waitFor(() => expect(maximized()).toBe(true));
  expect(windowMock.isMaximized).toHaveBeenCalledTimes(2);
  cleanup();
  expect(unlisten).toHaveBeenCalledOnce();
});

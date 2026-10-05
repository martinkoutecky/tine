import { beforeEach, expect, it, vi } from "vitest";
import { setToasts, toasts } from "../toasts";

const windowMock = vi.hoisted(() => ({
  isMaximized: vi.fn(async () => false),
  onResized: vi.fn(async (_callback?: () => void) => () => {}),
  minimize: vi.fn(async () => {}),
  toggleMaximize: vi.fn(async () => {}),
  close: vi.fn(async () => {}),
  startResizeDragging: vi.fn(async (_direction?: string) => {}),
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

import { render } from "solid-js/web";
import { ResizeGrips, WindowControls, installWindowChrome, maximized } from "./WindowChrome";

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

// UI-OG-C5-P6-WINDOW: a rejected native window request reaches the user (I-9), and a
// listener registered after cleanup is released (I-20/I-21).
it.each([
  ["Minimize", "minimize", 0],
  ["Maximize or Restore", "toggleMaximize", 1],
  ["Close", "close", 2],
] as const)("reports a rejected %s request with fixed text", async (_title, method, index) => {
  windowMock[method].mockRejectedValueOnce(new Error("private window title"));
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <WindowControls />, host);
  try {
    host.querySelectorAll<HTMLButtonElement>("button.win-btn")[index].click();
    await vi.waitFor(() => expect(toasts().some((toast) => toast.message === "Couldn't change the window.")).toBe(true));
    expect(toasts().map((toast) => toast.message).join(" ")).not.toContain("private window title");
  } finally { dispose(); host.remove(); }
});

it("reports a rejected resize-drag request", async () => {
  windowMock.startResizeDragging.mockRejectedValueOnce(new Error("no resize"));
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <ResizeGrips />, host);
  try {
    host.querySelector(".grip-n")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    await vi.waitFor(() => expect(toasts().some((toast) => toast.message === "Couldn't change the window.")).toBe(true));
  } finally { dispose(); host.remove(); }
});

it("reports a rejected resize-listener registration", async () => {
  windowMock.onResized.mockRejectedValueOnce(new Error("no listener"));
  const cleanup = installWindowChrome();
  try {
    await vi.waitFor(() => expect(toasts().some((toast) => toast.message === "Couldn't read the window state.")).toBe(true));
  } finally { cleanup(); }
});

it("releases a resize listener that registers after the installer was cleaned up", async () => {
  const unlisten = vi.fn();
  let register: (u: () => void) => void = () => {};
  windowMock.onResized.mockImplementationOnce(() => new Promise<() => void>((resolve) => { register = resolve; }));
  const cleanup = installWindowChrome();
  cleanup();
  register(unlisten);
  await vi.waitFor(() => expect(unlisten).toHaveBeenCalledOnce());
});

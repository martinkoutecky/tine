import { expect, it, vi } from "vitest";
import { setToasts, toasts } from "../toasts";

const windowMock = vi.hoisted(() => ({
  isMaximized: vi.fn(async () => false),
  onResized: vi.fn(async () => () => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => windowMock }));

import { installWindowChrome } from "./WindowChrome";

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

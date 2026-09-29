import { expect, it, vi } from "vitest";
import { setToasts, toasts } from "./toasts";

const backendMock = vi.hoisted(() => ({
  debugInfo: vi.fn(async () => ({ enabled: true, path: "/tmp/tine-debug.log" })),
  debugLog: vi.fn(async () => {}),
}));
vi.mock("./backend", () => ({ backend: () => backendMock }));

import { initDebug } from "./debug";

it("reports a failed debug-log write and stops retrying", async () => {
  setToasts([]);
  backendMock.debugLog.mockRejectedValueOnce(new Error("private graph detail"));
  await initDebug();
  await vi.waitFor(() => expect(toasts().some((toast) => toast.message === "Debug log unavailable.")).toBe(true));
  expect(toasts().map((toast) => toast.message).join(" ")).not.toContain("private graph detail");
  expect(backendMock.debugLog).toHaveBeenCalledTimes(1);
});

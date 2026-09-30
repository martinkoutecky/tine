import { afterEach, expect, it, vi } from "vitest";
const debug = vi.hoisted(() => vi.fn());
vi.mock("./backend", () => ({ backend: () => ({ graphBindingGeneration: () => 1 }) }));
vi.mock("./debug", () => ({ dbg: debug }));
import { reportUiFailure } from "./uiFailure";
import { setToasts, toasts } from "./toasts";
afterEach(() => { setToasts([]); debug.mockClear(); });
it("logs typed read details and keeps one sticky visible error per family", () => {
  reportUiFailure("custom-css", { kind: "failed", message: "read denied" });
  reportUiFailure("custom-css", { kind: "failed", message: "still denied" });
  expect(toasts()).toEqual([expect.objectContaining({ sticky: true, kind: "error", message: expect.stringContaining("No custom CSS") })]);
  expect(debug).toHaveBeenCalledWith(expect.stringContaining('"message":"read denied"'));
});
it("reports an opaque circular cause without throwing from feedback", () => {
  const cause: { self?: unknown } = {}; cause.self = cause;
  expect(() => reportUiFailure("config-read", cause)).not.toThrow();
  expect(toasts()).toEqual([expect.objectContaining({ sticky: true, kind: "error" })]);
});

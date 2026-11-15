import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { initCopySettings } from "./copySettings";

afterEach(() => vi.restoreAllMocks());
it("startup reads only the collapsed-property and reference-navigation preferences", async () => {
  const read = vi.spyOn(backend(), "getAppBool").mockImplementation(async (_key, fallback) => fallback);
  await initCopySettings();
  expect(read.mock.calls).toEqual([["copy_strip_collapsed", true], ["ref_click_zoom", false]]);
});

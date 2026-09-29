// OG-TOAST sweep: a reference rendered before the window's graph is bound
// (right sidebar restored at launch) must not ask the backend for icons and
// raise an error toast for the backend's no-graph refusal.
import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { pageIcon } from "./pageIconBatch";
import { setToasts, toasts } from "./toasts";

afterEach(() => { vi.restoreAllMocks(); setToasts([]); });

it("an unbound window fetches no page icons and reports nothing; a bound one fetches them", async () => {
  setToasts([]);
  const generation = vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(0);
  const icons = vi.spyOn(backend(), "pageIcons").mockRejectedValue(new Error("missing-graph-binding"));
  expect(pageIcon("Launch Ref")).toBe("");
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(icons).not.toHaveBeenCalled();
  expect(toasts()).toEqual([]);
  generation.mockReturnValue(1);
  icons.mockResolvedValue({ "Launch Ref": "*" });
  pageIcon("Launch Ref");
  await vi.waitFor(() => expect(icons).toHaveBeenCalledWith(["Launch Ref"]));
});

import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { copyGuideIntoGraph, maybeShowGuideAnnouncement } from "./guide";
import { dismissToast, graphMeta, pageInventoryRev, setGraphMeta, setToasts, toasts } from "./ui";

async function seedMeta(root: string) {
  const meta = await backend().loadGraph("");
  if (meta.kind === "focused_existing") throw new Error("mock graph unexpectedly focused another window");
  setGraphMeta({ ...meta.meta, root, guide_announced: false });
  setToasts([]);
}

afterEach(() => {
  setToasts([]);
  setGraphMeta(null);
  vi.restoreAllMocks();
});

describe("guide announcement", () => {
  it("shows once when guide_announced is unset and Dismiss persists the flag", async () => {
    const setFlag = vi.spyOn(backend(), "setGuideAnnounced").mockResolvedValue();
    await seedMeta("/mock/guide-dismiss");

    maybeShowGuideAnnouncement();
    expect(toasts()).toHaveLength(1);
    expect(toasts()[0].message).toBe("New: in-app Guide \u2014 learn Sheets, formulas & queries.");

    dismissToast(toasts()[0].id);
    expect(setFlag).toHaveBeenCalledWith(true);
    expect(graphMeta()?.guide_announced).toBe(true);
  });

  it("Open Guide action also persists the flag when the toast closes", async () => {
    const setFlag = vi.spyOn(backend(), "setGuideAnnounced").mockResolvedValue();
    await seedMeta("/mock/guide-open");

    maybeShowGuideAnnouncement();
    const toast = toasts()[0];
    toast.action?.run();
    dismissToast(toast.id);

    expect(setFlag).toHaveBeenCalledWith(true);
    expect(graphMeta()?.guide_announced).toBe(true);
  });
});

describe("guide copy inventory", () => {
  it("refreshes canonical page inventory when the backend creates guide pages", async () => {
    const copy = vi.spyOn(backend(), "copyGuideIntoGraph").mockResolvedValue({
      name: "tine-guide/Tine Guide",
      created: true,
      created_pages: ["tine-guide/Tine Guide"],
    });
    const before = pageInventoryRev();
    await copyGuideIntoGraph("Tine-guide/Tine Guide");
    expect(copy).toHaveBeenCalledWith("Tine Guide");
    expect(pageInventoryRev()).toBeGreaterThan(before);
  });

  it("does not refresh page inventory for an assets-only Guide repair", async () => {
    vi.spyOn(backend(), "copyGuideIntoGraph").mockResolvedValue({
      name: "tine-guide/Tine Guide",
      created: true,
      created_pages: [],
      copied_assets: ["assets/guide-image.png"],
    });
    const before = pageInventoryRev();
    await copyGuideIntoGraph("Tine-guide/Tine Guide");
    expect(pageInventoryRev()).toBe(before);
  });
});

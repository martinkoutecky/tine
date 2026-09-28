import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { copyGuideIntoGraph, ensureGuidePagesLoaded, maybeShowGuideAnnouncement } from "./guide";
import { dismissToast, setToasts, toasts } from "./toasts";
import { graphMeta, pageInventoryRev, setGraphMeta } from "./graphSession";
import { resetStore } from "./document";
import type { GuidePage } from "./types";

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
  it("starts a fresh Guide read when the old graph's pending read is retired", async () => {
    let finish!: (pages: GuidePage[]) => void;
    const read = vi.spyOn(backend(), "guidePages")
      .mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }))
      .mockResolvedValueOnce([]);
    await seedMeta("/mock/guide-read-old");
    const first = ensureGuidePagesLoaded(true);
    const oldMeta = graphMeta()!;
    resetStore();
    setGraphMeta({ ...oldMeta, root: "/mock/guide-read-new" });
    const second = ensureGuidePagesLoaded();
    expect(read).toHaveBeenCalledTimes(2);
    finish([]);
    await Promise.all([first, second]);
  });

  it("does not announce the new graph when an old graph toast is dismissed", async () => {
    const setFlag = vi.spyOn(backend(), "setGuideAnnounced").mockResolvedValue();
    await seedMeta("/mock/guide-old");
    maybeShowGuideAnnouncement();
    const oldToast = toasts()[0];
    const oldMeta = graphMeta()!;
    resetStore();
    setGraphMeta({ ...oldMeta, root: "/mock/guide-new", guide_announced: false });
    dismissToast(oldToast.id);
    expect(graphMeta()?.guide_announced).toBe(false);
    expect(setFlag).not.toHaveBeenCalled();
  });

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

  it("reports a failed announcement write and restores the flag", async () => {
    vi.spyOn(backend(), "setGuideAnnounced").mockRejectedValue(new Error("disk full"));
    await seedMeta("/mock/guide-failed-write");
    maybeShowGuideAnnouncement();
    dismissToast(toasts()[0].id);
    await Promise.resolve();
    await Promise.resolve();
    expect(graphMeta()?.guide_announced).toBe(false);
    expect(toasts().some((toast) => toast.kind === "error")).toBe(true);
  });
});

describe("guide copy inventory", () => {
  it("does not navigate when a Guide copy from the old graph completes", async () => {
    let finish!: (value: { name: string; created: boolean; created_pages: string[] }) => void;
    vi.spyOn(backend(), "copyGuideIntoGraph").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    await seedMeta("/mock/guide-copy-old");
    const pending = copyGuideIntoGraph("Tine-guide/Tine Guide");
    const oldMeta = graphMeta()!;
    resetStore();
    setGraphMeta({ ...oldMeta, root: "/mock/guide-copy-new" });
    const before = pageInventoryRev();
    finish({ name: "tine-guide/Tine Guide", created: true, created_pages: ["tine-guide/Tine Guide"] });
    await pending;
    expect(pageInventoryRev()).toBe(before);
  });

  it("refreshes canonical page inventory when the backend creates guide pages", async () => {
    const copy = vi.spyOn(backend(), "copyGuideIntoGraph").mockResolvedValue({
      name: "tine-guide/Tine Guide",
      created: true,
      created_pages: ["tine-guide/Tine Guide"],
    });
    const before = pageInventoryRev();
    await copyGuideIntoGraph("Tine-guide/Tine Guide");
    expect(copy).toHaveBeenCalledWith("Tine Guide", "replace-page");
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

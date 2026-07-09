import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { maybeShowGuideAnnouncement } from "./guide";
import { dismissToast, graphMeta, setGraphMeta, setToasts, toasts } from "./ui";

async function seedMeta(root: string) {
  const meta = await backend().loadGraph("");
  setGraphMeta({ ...meta, root, guide_announced: false });
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

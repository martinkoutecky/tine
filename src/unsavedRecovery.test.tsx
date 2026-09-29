// GH #540 (master dc3f2104b): a close with unsaved pages names them, and "No"
// opens a recovery panel that retries the ordinary save, opens the page, or
// copies the draft. A failed save's toast leads to the same panel.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { initParser } from "./render/parse";
import { flushAll, isDirty, loadFeed, pageByName, resetStore, setRaw } from "./document";
import { safeClose } from "./App";
import { UnsavedRecovery } from "./components/UnsavedRecovery";
import { closeUnsavedRecovery, unsavedRecoveryOpen } from "./unsavedRecovery";
import { setToasts, toasts } from "./toasts";
import type { BlockDto } from "./types";

const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });
const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };

let failing = true;
beforeAll(() => initParser());
beforeEach(() => {
  resetStore();
  setToasts([]);
  failing = true;
  vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => {
    if (failing) throw new Error("disk full");
    return { ok: entries.map((_, i) => `saved-${i}`) };
  });
  loadFeed([{ id: "pages/P.md", name: "P", title: "P", kind: "page", pre_block: null, rev: "r1",
    blocks: [block("p1", "first", [block("p2", "child")])] } as never]);
  setRaw("p1", "typed draft");
});
afterEach(() => {
  closeUnsavedRecovery();
  safeClose.reset();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("unsaved-changes recovery (GH #540)", () => {
  it("the close prompt names the page, and No opens the recovery panel", async () => {
    const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(false);
    expect(await safeClose.prepare()).toBe("rejected");
    expect(confirm).toHaveBeenCalledOnce();
    expect(confirm.mock.calls[0][0]).toContain("• P — Not saved");
    expect(unsavedRecoveryOpen()).toBe(true);
    expect(isDirty("P")).toBe(true);
  });

  it("shows the draft as source text, copies it, and retries the ordinary save", async () => {
    await flushAll();
    const write = vi.spyOn(backend(), "writeText").mockResolvedValue();
    const toast = toasts().find((t) => t.action?.label === "Review unsaved");
    expect(toast?.sticky).toBe(true);
    toast!.action!.run(); // the failed-save toast leads to the panel
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <UnsavedRecovery />, root);
    try {
      expect(root.querySelector("h3")?.textContent).toBe("P — Not saved");
      expect(root.querySelector("pre")?.textContent).toBe("- typed draft\n\t- child");
      const button = (label: string) => [...root.querySelectorAll("button")].find((b) => b.textContent === label)!;
      button("Copy draft").click();
      await flush();
      expect(write).toHaveBeenCalledWith("- typed draft\n\t- child");
      expect(isDirty("P")).toBe(true); // copying never acknowledges a save

      failing = false;
      button("Retry saving").click();
      await vi.waitFor(() => expect(root.textContent).toContain("All pending changes saved"));
      expect(isDirty("P")).toBe(false);
      expect(toasts().some((t) => t.action?.label === "Review unsaved")).toBe(false);
    } finally {
      dispose();
    }
  });

  it("belongs to the graph it was opened for", async () => {
    vi.spyOn(backend(), "confirm").mockResolvedValue(false);
    await safeClose.prepare();
    expect(unsavedRecoveryOpen()).toBe(true);
    resetStore(); // graph switch
    expect(unsavedRecoveryOpen()).toBe(false);
    expect(pageByName("P")).toBeUndefined();
  });
});

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { createComputed, createRoot } from "solid-js";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { deletePage, pageByName, resetStore } from ".";
import { loadSingle } from "./workingSet";
import { paneRouter, removePageTargetAcrossPanes, resetPaneLayoutToSingle } from "../panes";
import { bindTestHost } from "./host/wiring.test.support";
import { setToasts, toasts } from "../toasts";

// GH #376 (master 950ca6ccb): deleting the open page on Android passed through
// a black frame. The page left the working set first and its pane route was
// retired a promise turn later, so the pane rendered a route that named a page
// that no longer existed. Route retirement now runs inside the durable delete,
// before the page is purged.
beforeAll(() => initParser());

beforeEach(async () => {
  resetStore();
  resetPaneLayoutToSingle({
    tabs: [{ history: [{ kind: "journals" }, { kind: "page", name: "Doomed", pageKind: "page" }], pos: 1, pinned: false }],
    activeIndex: 0,
  });
  loadSingle({
    name: "Doomed", kind: "page", title: "Doomed", pre_block: null,
    blocks: [{ id: "d1", raw: "about to go", collapsed: false, children: [] }],
  });
  await bindTestHost();
});
afterEach(() => {
  resetStore();
  vi.restoreAllMocks();
});

describe("page delete retires routes before purging the page (GH #376)", () => {
  it("never shows a pane route that names a page already purged", async () => {
    const remove = vi.spyOn(backend(), "pageDelete");
    const orphanedFrames: string[] = [];
    const dispose = createRoot((dispose) => {
      createComputed(() => {
        const route = paneRouter("main").route();
        if (route.kind === "page" && route.name === "Doomed" && !pageByName("Doomed")) {
          orphanedFrames.push("route names a purged page");
        }
      });
      return dispose;
    });
    try {
      const target = { name: "Doomed", pageKind: "page" as const };
      // Exactly what the page menu's Delete action does.
      const ok = await deletePage("Doomed", "page", undefined, () => removePageTargetAcrossPanes(target));
      if (ok) removePageTargetAcrossPanes(target); // idempotent; the pre-fix menu did only this
      expect(ok).toBe(true);
      expect(remove).toHaveBeenCalledOnce();
      expect(pageByName("Doomed")).toBeUndefined();
      expect(paneRouter("main").route().kind).toBe("journals");
      expect(orphanedFrames).toEqual([]);
    } finally {
      dispose();
      remove.mockRestore();
    }
  });

  it("does not retire routes when the disk delete fails, and survives a throwing retirement", async () => {
    const refused = vi.spyOn(backend(), "pageDelete").mockResolvedValueOnce("refused");
    const retire = vi.fn();
    expect(await deletePage("Doomed", "page", undefined, retire)).toBe(false);
    expect(retire).not.toHaveBeenCalled();
    expect(pageByName("Doomed")).toBeDefined();
    refused.mockRestore();

    const failing = vi.spyOn(backend(), "pageDelete").mockRejectedValue(new Error("disk"));
    expect(await deletePage("Doomed", "page", undefined, retire)).toBe(false);
    expect(retire).not.toHaveBeenCalled();
    expect(pageByName("Doomed")).toBeDefined();
    failing.mockRestore();

    const ok = vi.spyOn(backend(), "pageDelete");
    expect(await deletePage("Doomed", "page", undefined, () => { throw new Error("ui"); })).toBe(true);
    expect(pageByName("Doomed")).toBeUndefined();
    ok.mockRestore();
  });

  // Finding B: the host answers a delete only once it published. A deletion
  // that is unwritten, unconfirmed or superseded keeps the page and says why;
  // it never reads as done (the file may still be there) or as silent.
  it.each([
    ["pending", "once its file can be written"],
    ["uncertain", "couldn't confirm"],
    ["superseded", "changed by a later edit"],
  ] as const)("keeps the page and explains a %s delete", async (operation, says) => {
    setToasts([]);
    const remove = vi.spyOn(backend(), "pageDelete").mockResolvedValueOnce(operation);
    const retire = vi.fn();
    expect(await deletePage("Doomed", "page", undefined, retire)).toBe(false);
    expect(remove).toHaveBeenCalledOnce();
    expect(retire).not.toHaveBeenCalled();
    expect(pageByName("Doomed")).toBeDefined();
    expect(toasts().map((toast) => toast.message).join("\n")).toContain(says);
  });
});

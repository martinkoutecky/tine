// GH #623 (comment 17): the image hover "Trash" drops the reference from the
// block (a debounced page save) and then asks the backend to trash the file.
// The backend refuses to trash a file the PUBLISHED graph still references, so
// a trash fired before the block's save landed failed with
// "asset is referenced; refresh the orphan inventory". The reference removal
// must be durable first; a file still used elsewhere is kept with a plain
// message, decided by the typed outcome and not by the error text.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

vi.mock("../assetCache", async (orig) => ({
  ...(await orig<typeof import("../assetCache")>()),
  acquireAssetBlob: () => Promise.resolve({ url: "blob:asset", release: () => {} }),
}));

import { AstBody } from "./body";
import { initParser } from "./parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { setGraphMeta } from "../graphSession";
import { backend } from "../backend";
import { toasts, setToasts } from "../toasts";

beforeAll(async () => {
  await initParser();
});
afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  setToasts([]);
  document.body.innerHTML = "";
});

const IMG = "![a](../assets/x.png)";

async function mount(raw: string) {
  loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: null, blocks: [{ id: "body", raw, collapsed: false, children: [] }], format: "md" });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => AstBody({ raw, blockId: "body" }), host);
  for (let i = 0; i < 5; i++) await Promise.resolve();
  return { host, dispose };
}

/** A backend that behaves like the real one at the seam this bug lives on: it
 *  remembers what the last page save wrote, and trashes an asset only when that
 *  published text no longer names it (`check_orphan_asset`). `elsewhere` models
 *  a second page still referencing the file. */
function fakeBackend(elsewhere: boolean, saveOk = true) {
  let published = IMG;
  const saves: string[] = [];
  vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => {
    if (!saveOk) return { failed: { index: 0, family: "conflict", undoFailed: [] } };
    published = entries[0].page.blocks.map((b) => b.raw).join("\n");
    saves.push(published);
    return { ok: entries.map(() => "rev") };
  });
  const trashAsset = vi.spyOn(backend(), "trashAsset").mockImplementation(async () => {
    if (published.includes("x.png") || elsewhere) return "referenced" as never;
    return "trashed" as never;
  });
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  return { trashAsset, saves };
}

async function clickTrash(host: Element) {
  host.querySelector(".asset-action-trash")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  await vi.waitFor(() => expect(toasts().length).toBeGreaterThan(0));
  return toasts().map((t) => t.message);
}

describe("trashing an image asset", () => {
  it("makes the reference removal durable before the file is trashed", async () => {
    const { host, dispose } = await mount(`see ${IMG}`);
    const { trashAsset, saves } = fakeBackend(false);
    const messages = await clickTrash(host);
    expect(saves, "the block is saved without the image before the file moves").toEqual(["see"]);
    expect(trashAsset).toHaveBeenCalledTimes(1);
    expect(messages.join("\n")).toContain("moved to trash");
    expect(messages.join("\n")).not.toContain("Couldn't");
    expect(doc.byId.body.raw).toBe("see");
    dispose();
  });

  it("keeps a file that other places still use, says so plainly, and leaves the block edit in place", async () => {
    const { host, dispose } = await mount(`see ${IMG}`);
    const { trashAsset } = fakeBackend(true);
    const messages = await clickTrash(host);
    expect(trashAsset).toHaveBeenCalledTimes(1);
    expect(messages.join("\n")).toMatch(/removed from this block/i);
    expect(messages.join("\n")).toMatch(/still used elsewhere/i);
    expect(messages.join("\n")).not.toContain("refresh the orphan inventory");
    expect(doc.byId.body.raw).toBe("see");
    dispose();
  });

  it("trashes nothing while the block edit could not be saved", async () => {
    const { host, dispose } = await mount(`see ${IMG}`);
    const { trashAsset } = fakeBackend(false, false);
    const messages = await clickTrash(host);
    expect(trashAsset, "I-2: a reference that is not durably gone never frees its file").not.toHaveBeenCalled();
    expect(messages.join("\n")).toMatch(/not saved|couldn.t save/i);
    dispose();
  });
});

// C3 L16 (og c3s F6): an image resize or trash edits THE token that was clicked
// (located by its lsdoc source span and verified against its exact source
// text), never the first textual lookalike: not a duplicate earlier in the
// block, not one inside a code fence. And the asset file is trashed only after
// its reference was actually removed — a label with markup must not leave a
// dangling reference to a trashed file.
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
import { bindTestHost } from "../document/host/wiring.test.support";
import { openAsLoaded } from "../components/hostConflict.test.support";

beforeAll(async () => {
  await initParser();
});
afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  document.body.innerHTML = "";
});

async function mount(raw: string, bodyRaw = raw) {
  loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: null, blocks: [{ id: "body", raw, collapsed: false, children: [] }], format: "md" });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => AstBody({ raw: bodyRaw, blockId: "body" }), host);
  for (let i = 0; i < 5; i++) await Promise.resolve();
  return { host, dispose };
}

function resize(grip: Element) {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ width: 100 } as DOMRect);
  grip.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, clientX: 0 }));
  window.dispatchEvent(new MouseEvent("pointerup", { clientX: 0 }));
}

async function trash(button: Element) {
  const trashAsset = vi.spyOn(backend(), "trashAsset").mockResolvedValue("trashed");
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  for (let i = 0; i < 10; i++) await Promise.resolve();
  return trashAsset;
}

const IMG = "![a](../assets/x.png)";

describe("media token edits target the clicked token", () => {
  it("resizing the second of two identical images writes the second", async () => {
    const { host, dispose } = await mount(`${IMG} ${IMG}`);
    const grips = host.querySelectorAll(".img-resize-grip");
    expect(grips).toHaveLength(2);
    resize(grips[1]);
    expect(doc.byId.body.raw).toBe(`${IMG} ${IMG}{:width "100%"}`);
    dispose();
  });

  it("resizing never edits a lookalike inside a code fence", async () => {
    const raw = `\`\`\`\n${IMG}\n\`\`\`\n${IMG}`;
    const { host, dispose } = await mount(raw);
    const grips = host.querySelectorAll(".img-resize-grip");
    expect(grips).toHaveLength(1);
    resize(grips[0]);
    expect(doc.byId.body.raw).toBe(`\`\`\`\n${IMG}\n\`\`\`\n${IMG}{:width "100%"}`);
    dispose();
  });

  it("trashing an image whose label has markup removes its reference, then the file", async () => {
    // The reference removal publishes through the page host, which read the page shown.
    openAsLoaded(await bindTestHost());
    const { host, dispose } = await mount("see ![**b**](../assets/x.png) here");
    const trashAsset = await trash(host.querySelector(".asset-action-trash")!);
    expect(doc.byId.body.raw).toBe("see here");
    // The reference removal is saved before the file moves (GH #623), so the trash lands a few turns later.
    await vi.waitFor(() => expect(trashAsset).toHaveBeenCalledTimes(1));
    dispose();
  });

  it("a token that cannot be located in the block text trashes nothing", async () => {
    // Rendered text is not the block's raw (a stale render / an expansion).
    // Bound, so a save could succeed: the refusal is the token lookup's.
    openAsLoaded(await bindTestHost());
    const { host, dispose } = await mount("unrelated text", `see ${IMG}`);
    const trashAsset = await trash(host.querySelector(".asset-action-trash")!);
    expect(doc.byId.body.raw).toBe("unrelated text");
    expect(trashAsset, "I-2: no file is trashed while nothing dropped its reference").not.toHaveBeenCalled();
    dispose();
  });
});

describe("media resize gesture ownership", () => {
  it.each(["png", "mp4"].flatMap((ext) => ["pointercancel", "lostpointercapture", "unmount", "graph switch"].map((retire) => [ext, retire])))("%s resize ends on %s and cannot edit the next graph", async (ext, retire) => {
    const raw = `![a](../assets/x.${ext})`;
    vi.spyOn(backend(), "streamAsset").mockResolvedValue("https://assets.test/video.mp4");
    {
      resetStore();
      const { host, dispose } = await mount(raw);
      await vi.waitFor(() => expect(host.querySelector(".img-resize-grip")).not.toBeNull());
      vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ width: 100 } as DOMRect);
      const removed = vi.spyOn(window, "removeEventListener");
      const grip = host.querySelector(".img-resize-grip")!;
      grip.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerId: 7, clientX: 0 }));
      if (retire === "pointercancel") window.dispatchEvent(new PointerEvent("pointercancel", { pointerId: 7 }));
      if (retire === "lostpointercapture") grip.dispatchEvent(new PointerEvent("lostpointercapture", { pointerId: 7 }));
      if (retire === "unmount") dispose();
      resetStore();
      loadSingle({ name: "Test", kind: "page", title: "Test", pre_block: null, blocks: [{ id: "body", raw, collapsed: false, children: [] }] });
      window.dispatchEvent(new PointerEvent("pointermove", { pointerId: 7, clientX: 50 }));
      window.dispatchEvent(new PointerEvent("pointerup", { pointerId: 7 }));
      expect(doc.byId.body.raw, retire).toBe(raw);
      for (const event of ["pointermove", "pointerup", "pointercancel", "blur"]) {
        expect(removed.mock.calls.some(([name]) => name === event), `release ${event} on ${retire}`).toBe(true);
      }
      if (retire !== "unmount") dispose();
      host.remove();
    }
  });

  it("only the initiating pointer can finish a resize", async () => {
    const { host, dispose } = await mount(IMG);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ width: 100 } as DOMRect);
    host.querySelector(".img-resize-grip")!.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerId: 7 }));
    window.dispatchEvent(new PointerEvent("pointerup", { pointerId: 8 }));
    expect(doc.byId.body.raw).toBe(IMG);
    window.dispatchEvent(new PointerEvent("pointerup", { pointerId: 7 }));
    expect(doc.byId.body.raw).toContain('{:width "100%"}');
    dispose();
  });
});

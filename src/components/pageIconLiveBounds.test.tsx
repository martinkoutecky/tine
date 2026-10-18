import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { mkdtempSync, mkdirSync, writeFileSync, symlinkSync, readFileSync } from "node:fs";
import "../graph";
import { backend } from "../backend";
import { bindTestHost } from "../document/host/wiring.test.support";
import { loadSingle } from "../document/workingSet";
import { resetStore, setPageProperty } from "../document";
import { bumpDataRev } from "../graphSession";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import { acquireAssetBlob, clearAssetBlobCache, seedAssetBlob } from "../assetCache";
import { applyObservedAssetChanges } from "../assetRefresh";
import { PageIcon } from "./PageIcon";
import { NamespaceMacro } from "./Namespace";

const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aFOkAAAAASUVORK5CYII="), c => c.charCodeAt(0));
const settle = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };
let dispose: (() => void) | undefined;
let root: HTMLDivElement;
beforeEach(async () => {
  await bindTestHost();
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
  bumpDataRev();
  let serial = 0;
  vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: vi.fn(() => `blob:bounds-${++serial}`), revokeObjectURL: vi.fn() }));
  root = document.createElement("div"); document.body.append(root);
});
afterEach(() => { dispose?.(); dispose = undefined; clearAssetBlobCache(); resetStore(); resetPageIndex(); document.body.innerHTML = ""; vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const load = (value = "![](../assets/a.png)", name = "Image") => loadSingle({ id: "pages/Image.md", name, kind: "page", title: name, pre_block: `icon:: ${value}`, blocks: [] });

it.each(["logo.png", "unrelated.pdf"])("refreshes all icons on an asset event (%s), sharing each replacement", async event => {
  load("![](../assets/LOGO.PNG)");
  const read = vi.spyOn(backend(), "readAsset").mockResolvedValue(png);
  dispose = render(() => <><PageIcon value="![](../assets/LOGO.PNG)" name="Image" /><PageIcon value="![](../assets/LOGO.PNG)" name="Image" /></>, root);
  await vi.waitFor(() => expect(root.querySelectorAll("img")).toHaveLength(2));
  applyObservedAssetChanges([event, "other.png"]);
  await vi.waitFor(() => expect([...root.querySelectorAll("img")].map(img => img.getAttribute("src"))).toEqual(["blob:bounds-2", "blob:bounds-2"]));
  expect(read).toHaveBeenCalledTimes(2);
});

it("refreshes an in-assets symlink icon when its target changes", async () => {
  mkdirSync("artifacts/page-image-icons", { recursive: true });
  const graph = mkdtempSync("artifacts/page-image-icons/symlink-r5-");
  mkdirSync(`${graph}/assets`); writeFileSync(`${graph}/assets/actual.png`, png); symlinkSync("actual.png", `${graph}/assets/link.png`);
  load("![](../assets/link.png)");
  const read = vi.spyOn(backend(), "readAsset").mockImplementation(async rel => new Uint8Array(readFileSync(`${graph}/assets/${rel}`)));
  dispose = render(() => <PageIcon value="![](../assets/link.png)" name="Image" />, root);
  await vi.waitFor(() => expect(root.querySelector("img")?.getAttribute("src")).toBe("blob:bounds-1"));
  writeFileSync(`${graph}/assets/actual.png`, new Uint8Array([...png, 0])); applyObservedAssetChanges(["actual.png"]);
  await vi.waitFor(() => expect(root.querySelector("img")?.getAttribute("src")).toBe("blob:bounds-2"));
  expect(read).toHaveBeenCalledTimes(2);
});

it("updates namespace roots and descendants on unsaved edits and unloaded-page publication", async () => {
  load("![](../assets/a.png)", "Top/Child");
  const icons = vi.spyOn(backend(), "pageIcons").mockResolvedValue({ Top: "RT", "Top/Child": "![](../assets/a.png)" });
  vi.spyOn(backend(), "readAsset").mockResolvedValue(png);
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [{ key: "top/child", name: "Top/Child", is_journal: false, day: null, target: { kind: "existing", id: "pages/Image.md", others: [] } }] });
  await refreshPageIndex(); dispose = render(() => <NamespaceMacro root="Top" />, root);
  await vi.waitFor(() => expect(root.querySelectorAll("img[data-page-image-icon]")).toHaveLength(1));
  setPageProperty("Top/Child", "icon", "AB");
  expect(root.querySelector(".ns-macro-tree")?.textContent).toContain("AB");
  expect(root.querySelector("img[data-page-image-icon]")).toBeNull();
  icons.mockResolvedValue({ Top: "NEW" }); bumpDataRev();
  await vi.waitFor(() => expect(root.querySelector(".ns-macro-head")?.textContent).toContain("NEW"));
});

it.each([1024 * 1024, 1024 * 1024 + 1])("bounds icon files at 1 MiB (%i bytes) and shares the blob with block images", async size => {
  load(); const bytes = new Uint8Array(size); bytes.set(png);
  const read = vi.spyOn(backend(), "readAsset").mockResolvedValue(bytes);
  dispose = render(() => <><PageIcon value="![](../assets/a.png)" name="Image" /><PageIcon value="![](../assets/a.png)" name="Image" /></>, root);
  await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1)); await settle();
  if (size > 1024 * 1024) {
    expect(root.querySelector("img")).toBeNull(); expect(URL.createObjectURL).not.toHaveBeenCalled();
  } else {
    await vi.waitFor(() => expect(root.querySelectorAll("img")).toHaveLength(2));
    const block = await acquireAssetBlob("a.png");
    expect(block.url).toBe(root.querySelector("img")?.getAttribute("src")); expect(URL.createObjectURL).toHaveBeenCalledTimes(1); block.release();
  }
  expect(read).toHaveBeenCalledWith("a.png", 1024 * 1024);
});

it("refuses an oversized blob already cached by a block image", async () => {
  load(); const bytes = new Uint8Array(1024 * 1024 + 1); bytes.set(png); seedAssetBlob("a.png", bytes);
  dispose = render(() => <PageIcon value="![](../assets/a.png)" name="Image" />, root);
  await settle(); expect(root.querySelector("img")).toBeNull(); expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
});

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { openTineLink } from "./deepLinkNavigation";
import { copyTineLink } from "./components/blockLinkCopy";
import { pageLink, blockLink } from "./deepLinks";
import { DeepLinkGraphChoice } from "./components/DeepLinkGraphChoice";
import { ContextMenu } from "./components/ContextMenu";
import { openPageContextMenu, openContextMenu, closeContextMenu } from "./ui";
import { initParser } from "./render/parse";
import { loadSingle } from "./document/workingSet";
import { isDirty, resetStore, pageByName } from "./document";
import { bindTestHost } from "./document/host/wiring.test.support";
import { setGraphMeta, bumpGraphEpoch } from "./graphSession";
import { setToasts, toasts } from "./toasts";
import { invalidateBinding } from "./binding";
import { focusedRouter, resetPaneLayoutToSingle, splitPane, focusPane, paneRouter } from "./panes";
const id = "11111111-1111-4111-8111-111111111111";
const root = "/fixture";
const dto = { id: "pages/日本.md", name: "日本", kind: "page" as const, title: "日本", pre_block: null,
  blocks: [{ id, raw: `Linked\nid:: ${id}`, collapsed: false, children: [] }] };
let dispose = () => {};
beforeAll(initParser);
afterEach(() => { dispose(); closeContextMenu(); vi.restoreAllMocks(); delete backend().tineLinks;
  resetStore(); setGraphMeta(null); setToasts([]); document.body.innerHTML = "";
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 }); });
async function setup() {
  // Bound: any edit would reach the page host (`pageSubmit`).
  await bindTestHost();
  setGraphMeta({ root, pages_dir: "pages", journals_dir: "journals", assets_dir: "assets" } as any);
  loadSingle(dto);
  const api = backend();
  api.tineLinks = { identity: vi.fn(async () => id), scanKnownGraphs: vi.fn(async () => [{ root, name: dto.name, pageKind: dto.kind, path: dto.id }]),
    take: vi.fn(async () => []), subscribe: vi.fn(async () => () => {}), handoff: vi.fn(async () => false) };
  vi.spyOn(api, "getPageByPath").mockResolvedValue(dto);
  vi.spyOn(api, "resolveBlocks").mockResolvedValue([{ page: dto.name, kind: dto.kind, blocks: dto.blocks }]);
  const host = document.createElement("div"); document.body.append(host);
  dispose = render(() => <><ContextMenu /><DeepLinkGraphChoice /></>, host);
  return api;
}
describe("GH #181 literal menu and navigation boundaries", () => {
  it("copies a page link from Page actions without changing its content", async () => {
    const api = await setup(); const write = vi.spyOn(api, "writeText").mockResolvedValue();
    const save = vi.spyOn(api, "pageSubmit");
    const before = JSON.stringify(pageByName(dto.name));
    openPageContextMenu(10, 10, dto.name, "page", true);
    document.querySelector<HTMLButtonElement>('[data-page-action-id="copy-link"]')!.click();
    await vi.waitFor(() => expect(write).toHaveBeenCalledWith(pageLink(dto.name, id)));
    expect(JSON.stringify(pageByName(dto.name))).toBe(before); expect(save).not.toHaveBeenCalled(); expect(isDirty(dto.name)).toBe(false);
  });
  it("copies an already identified block from its literal menu", async () => {
    const api = await setup(); const write = vi.spyOn(api, "writeText").mockResolvedValue();
    openContextMenu(10, 10, id);
    [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((item) => item.textContent?.trim() === "Copy link")!.click();
    await vi.waitFor(() => expect(write).toHaveBeenCalledWith(blockLink(id)));
  });
  it("navigates an existing external page in the real focused router without saving", async () => {
    const api = await setup(); const save = vi.spyOn(api, "pageSubmit");
    await openTineLink({ kind: "url", url: pageLink(dto.name, id) });
    expect(focusedRouter().route()).toMatchObject({ kind: "page", name: dto.name, path: dto.id });
    expect(save).not.toHaveBeenCalled(); expect(isDirty(dto.name)).toBe(false);
  });
  it("opens a block by UUID with a zoom route, never changing collapsed content", async () => {
    const api = await setup(); const save = vi.spyOn(api, "pageSubmit");
    vi.mocked(api.tineLinks!.scanKnownGraphs).mockResolvedValue([{ root, name: dto.name, pageKind: dto.kind, path: dto.id, block: id }]);
    await openTineLink({ kind: "url", url: blockLink(id) });
    expect(focusedRouter().route()).toMatchObject({ kind: "page", name: dto.name, block: id });
    expect(save).not.toHaveBeenCalled(); expect(isDirty(dto.name)).toBe(false);
  });
  it("opens in the focused split pane while keeping the other pane's route", async () => {
    await setup();
    const before = paneRouter("main").route();
    const other = splitPane("main", "row")!;
    focusPane(other);
    await openTineLink({ kind: "url", url: pageLink(dto.name, id) });
    expect(paneRouter(other).route()).toMatchObject({ kind: "page", name: dto.name });
    expect(paneRouter("main").route()).toEqual(before);
  });
  it("reports unknown graphs and deleted pages while keeping the current route", async () => {
    const api = await setup(); const before = focusedRouter().route();
    vi.mocked(api.tineLinks!.scanKnownGraphs).mockRejectedValueOnce(new Error("Unknown graph"));
    await openTineLink({ kind: "url", url: pageLink("missing", id) });
    expect(focusedRouter().route()).toEqual(before); expect(toasts().at(-1)?.message).toContain("Unknown graph");
    vi.mocked(api.getPageByPath).mockResolvedValueOnce(null);
    await openTineLink({ kind: "url", url: pageLink(dto.name, id) });
    expect(focusedRouter().route()).toEqual(before); expect(toasts().at(-1)?.message).toContain("no longer exists");
  });
  it("asks which graph copy to use once, then remembers the explicit selection", async () => {
    const api = await setup();
    vi.mocked(api.tineLinks!.scanKnownGraphs).mockResolvedValue([{ root: "/copy", graphId: id }, { root, graphId: id }]);
    let remembered = "";
    vi.spyOn(api, "getAppString").mockImplementation(async () => remembered);
    const remember = vi.spyOn(api, "setAppString").mockImplementation(async (_key, value) => { remembered = value; });
    const first = openTineLink({ kind: "url", url: `tine://graph/${id}` });
    await vi.waitFor(() => expect(document.querySelector('[role="dialog"]')).not.toBeNull());
    const buttons = [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')];
    buttons.at(-1)!.focus();
    buttons.at(-1)!.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(buttons[0]);
    buttons[0].dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(buttons.at(-1));
    [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find((button) => button.textContent === root)!.click();
    await first; expect(remember).toHaveBeenCalledWith(`tine-link-choice:${id}`, root);
    await openTineLink({ kind: "url", url: `tine://graph/${id}` });
    await openTineLink({ kind: "url", url: blockLink(id) });
    expect(document.querySelector('[role="dialog"]')).toBeNull(); expect(remember).toHaveBeenCalledTimes(1);
  });
  it("hands a link to the window that already owns its graph", async () => {
    const api = await setup(); vi.mocked(api.tineLinks!.handoff).mockResolvedValue(true);
    const before = focusedRouter().route();
    await openTineLink({ kind: "url", url: pageLink(dto.name, id) });
    expect(focusedRouter().route()).toEqual(before); expect(api.getPageByPath).not.toHaveBeenCalled();
  });
  it.each([null, "Graph read failed"])("refuses an unavailable target in the remembered graph copy (%s) instead of using another copy", async (error) => {
    const api = await setup();
    const before = focusedRouter().route();
    vi.mocked(api.tineLinks!.scanKnownGraphs).mockResolvedValue([
      { root: "/another-copy", graphId: id, name: dto.name, path: dto.id },
      { root, graphId: id, name: dto.name, path: null, error },
    ]);
    vi.spyOn(api, "getAppString").mockResolvedValue(root);
    await openTineLink({ kind: "url", url: pageLink(dto.name, id) });
    expect(focusedRouter().route()).toEqual(before);
    expect(api.tineLinks!.handoff).not.toHaveBeenCalled();
    expect(toasts().at(-1)?.message).toContain(error ?? "target not found in the chosen graph copy");
  });
  // A graph switch moves the binding (switchGraph -> resetStore -> invalidateBinding,
  // and loadGraph's new backend binding generation) and repaints (graphEpoch).
  const switchGraph = () => { invalidateBinding(); bumpGraphEpoch(); };
  it("cancels stale identity copies and resolutions after a graph switch", async () => {
    const api = await setup(); const write = vi.spyOn(api, "writeText");
    vi.mocked(api.tineLinks!.identity).mockImplementation(async () => { switchGraph(); return id; });
    await copyTineLink({ page: dto.name }); expect(write).not.toHaveBeenCalled();
    const before = focusedRouter().route();
    vi.mocked(api.tineLinks!.scanKnownGraphs).mockImplementation(async () => { switchGraph(); return [{ root }]; });
    await openTineLink({ kind: "url", url: pageLink(dto.name, id) }); expect(focusedRouter().route()).toEqual(before);
  });
  it("finishes a link copy across a display-only repaint of the same graph (R4)", async () => {
    const api = await setup(); const write = vi.spyOn(api, "writeText");
    vi.mocked(api.tineLinks!.identity).mockImplementation(async () => { bumpGraphEpoch(); return id; });
    await copyTineLink({ page: dto.name });
    expect(write).toHaveBeenCalledWith(pageLink(dto.name, id));
  });
});

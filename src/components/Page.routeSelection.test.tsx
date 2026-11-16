import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore, selectBlock, selectedIds, undo, extendFeedForScroll, flushAll } from "../document";
import { pageToDto } from "../document/convert";
import { bindTestHost } from "../document/host/wiring.test.support";
import { installDiskHost } from "../diskHost.test.support";
import { mainPaneRouter, resetTabsToJournals, type PaneRouter } from "../router";
import { PaneContext, resetPaneLayoutToSingle, setFocusedPaneId, splitPane, paneRouter } from "../panes";
import { endEdit, noteSurfaceFocused } from "../editorController";
import { installKeybindings } from "../keybindings";
import { closeSwitcher, openSwitcher, setRecentPages } from "../ui";
import { PageView } from "./Page";
import { QuickSwitcher } from "./QuickSwitcher";
import { SurfaceContext } from "./Block";
import type { PageDto, PageRead } from "../types";
import { journalTitle, localDayKey } from "../journal";

const page = (name: string): PageRead => ({
  id: `pages/${name}.md`, rev: "baseline", name, title: name, kind: "page", pre_block: null,
  blocks: [{ id: `${name}-root`, raw: `${name} visible content`, children: [], collapsed: false },
    { id: `${name}-link`, raw: "[[Other]]", children: [], collapsed: false }],
});
const disposers: (() => void)[] = [];
beforeEach(() => {
  vi.spyOn(backend(), "getPage").mockImplementation(async name => page(name));
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => page(path.slice(6, -3)));
  disposers.push(installKeybindings());
  noteSurfaceFocused("main");
});
afterEach(() => {
  for (const dispose of disposers.splice(0).reverse()) dispose();
  closeSwitcher(); setRecentPages([]);
  endEdit("blur"); resetStore(); resetPaneLayoutToSingle(); resetTabsToJournals();
  vi.restoreAllMocks(); document.body.replaceChildren(); vi.unstubAllGlobals();
});

async function mounted(router: PaneRouter = mainPaneRouter) {
  router.replaceActiveRoute({ kind: "page", name: "Cold", pageKind: "page" });
  const host = document.createElement("div"); document.body.append(host);
  const surface = router.paneId === "main" ? "main" : `pane:${router.paneId}`;
  disposers.push(render(() => <PaneContext.Provider value={{ paneId: router.paneId, router }}>
    <SurfaceContext.Provider value={surface}><PageView /></SurfaceContext.Provider>
  </PaneContext.Provider>, host));
  await vi.waitFor(() => expect(host.textContent).toContain("Cold visible content"));
  return host;
}
function press(key: string) {
  document.body.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}

// Ifods: navigation without an outside click must not leave Delete aimed at a hidden page.
it.each(["link", "palette", "back", "forward", "g j"])("Delete after %s leaves the departed page's blocks intact", async entry => {
  const host = await mounted();
  if (entry === "back") mainPaneRouter.openPage("Other", "page", { inPlace: true });
  if (entry === "forward") {
    mainPaneRouter.openPage("Other", "page", { inPlace: true }); mainPaneRouter.goBack();
  }
  const source = entry === "back" ? "Other" : "Cold";
  await vi.waitFor(() => expect(host.textContent).toContain(`${source} visible content`));
  const files = new Map([page("Cold"), page("Other")].map(dto => [dto.id, dto]));
  const writes = vi.fn((path: string, dto: PageDto) => {
    files.set(path, { ...structuredClone(dto), id: path, rev: "saved" }); return "saved";
  });
  installDiskHost(await bindTestHost(), { read: path => files.get(path) ?? null, write: writes });
  selectBlock(`${source}-root`);
  expect(selectedIds()).toEqual([`${source}-root`]);
  const before = pageToDto(source);
  const fileBefore = structuredClone(files.get(`pages/${source}.md`)!.blocks);
  if (entry === "link") host.querySelector<HTMLAnchorElement>("a.page-ref")!.click();
  if (entry === "palette") {
    const palette = document.createElement("div"); document.body.append(palette);
    disposers.push(render(() => <QuickSwitcher />, palette));
    setRecentPages([{ name: "Other", kind: "page", path: "pages/Other.md" }]);
    openSwitcher({ mode: "pages" });
    const row = await vi.waitFor(() => {
      const row = [...palette.querySelectorAll<HTMLElement>(".switcher-row")].find(row => row.textContent?.includes("Other"));
      expect(row).toBeDefined(); return row!;
    });
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
  }
  if (entry === "back") mainPaneRouter.goBack();
  if (entry === "forward") mainPaneRouter.goForward();
  if (entry === "g j") { press("g"); press("j"); }
  await vi.waitFor(() => expect(host.textContent).not.toContain(`${source} visible content`));
  press("Delete"); await flushAll();
  expect(files.get(`pages/${source}.md`)!.blocks).toEqual(fileBefore);
  expect(pageToDto(source)?.blocks).toEqual(before?.blocks);
  expect(writes).not.toHaveBeenCalled();
  expect(selectedIds()).toEqual([]);
  undo(); expect(pageToDto(source)?.blocks).toEqual(before?.blocks);
});

it("changing the other pane preserves selection; changing its own pane retires it", async () => {
  const right = splitPane("main", "row")!;
  const left = await mounted();
  const router = paneRouter(right), view = await mounted(router);
  setFocusedPaneId(right); noteSurfaceFocused(`pane:${right}`); selectBlock("Cold-root");
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Other", pageKind: "page" });
  await vi.waitFor(() => expect(left.textContent).toContain("Other visible content"));
  expect(selectedIds()).toEqual(["Cold-root"]);
  const before = pageToDto("Cold");
  router.replaceActiveRoute({ kind: "page", name: "Other", pageKind: "page" });
  await vi.waitFor(() => expect(view.textContent).toContain("Other visible content"));
  press("Backspace");
  expect(pageToDto("Cold")?.blocks).toEqual(before?.blocks);
  expect(selectedIds()).toEqual([]);
});

it("unmounting the selected pane retires its selection before Delete", async () => {
  await mounted();
  selectBlock("Cold-root");
  const before = pageToDto("Cold");
  disposers.pop()!();
  press("Delete");
  expect(pageToDto("Cold")?.blocks).toEqual(before?.blocks);
  expect(selectedIds()).toEqual([]);
});

it("paging and scrolling journals keeps the still-mounted selection", async () => {
  vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });
  const journal = (offset: number): PageRead => {
    const date = new Date(); date.setDate(date.getDate() - offset);
    return { ...page(journalTitle(date)), kind: "journal" };
  };
  const today = journal(0), yesterday = journal(1), older = journal(2);
  vi.spyOn(backend(), "journalFeedPage").mockImplementation(async (_limit, before) => ({
    pages: before == null ? [today, yesterday] : [older], as_of_day: localDayKey(),
    done: before != null, next_before_day: before == null ? localDayKey() - 2 : null,
  }));
  mainPaneRouter.resetTabsToJournals();
  const host = document.createElement("div"); document.body.append(host);
  disposers.push(render(() => <PageView />, host));
  await vi.waitFor(() => expect(host.textContent).toContain(`${today.name} visible content`));
  selectBlock(`${today.name}-root`); await extendFeedForScroll(); host.dispatchEvent(new Event("scroll"));
  await vi.waitFor(() => expect(host.textContent).toContain(`${older.name} visible content`));
  expect(host.textContent).toContain(`${today.name} visible content`);
  expect(selectedIds()).toEqual([`${today.name}-root`]);
});

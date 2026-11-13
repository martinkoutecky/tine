import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore, selectBlock, selectedIds, undo, extendFeedForScroll, mergeSelection, moveSelection } from "../document";
import { pageToDto } from "../document/convert";
import { mainPaneRouter, resetTabsToJournals, type PaneRouter } from "../router";
import { PaneContext, resetPaneLayoutToSingle, setFocusedPaneId, splitPane, paneRouter } from "../panes";
import { endEdit, noteSurfaceFocused } from "../editorController";
import { installKeybindings, runGlobalCommand } from "../keybindings";
import { applySidebarSession, closeSwitcher, openSwitcher, setRecentPages, setRightSidebar } from "../ui";
import { PageView } from "./Page";
import { QuickSwitcher } from "./QuickSwitcher";
import { SurfaceContext } from "./Block";
import { RightSidebar } from "./RightSidebar";
import { maybeEditorIn } from "../tests/editorDriver";
import { dispatchFocusedEditorCommand } from "../editorCommandBridge";
import type { PageRead } from "../types";
import { journalTitle, localDayKey } from "../journal";

const page = (name: string): PageRead => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null,
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
  setRightSidebar([]); applySidebarSession({ right: false, items: [] });
  endEdit("blur"); resetStore(); resetPaneLayoutToSingle(); resetTabsToJournals();
  vi.restoreAllMocks(); document.body.replaceChildren();
  vi.unstubAllGlobals();
});

async function mounted(router: PaneRouter = mainPaneRouter) {
  router.replaceActiveRoute({ kind: "page", name: "Cold", pageKind: "page" });
  const host = document.createElement("div"); document.body.append(host);
  const surface = router.paneId === "main" ? "main" : `pane:${router.paneId}`;
  const dispose = render(() => <PaneContext.Provider value={{ paneId: router.paneId, router }}>
    <SurfaceContext.Provider value={surface}><PageView /></SurfaceContext.Provider>
  </PaneContext.Provider>, host);
  disposers.push(dispose);
  await vi.waitFor(() => expect(host.textContent).toContain("Cold visible content"));
  return { host, dispose };
}
function pressDelete() {
  document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true }));
}

// Port of the cold-selection lane's literal routed-page departure probe.
// Ifods deliberately owns selection by route, including navigation without an outside click.
it("leaving a routed page releases its selection", async () => {
  const { host } = await mounted();
  selectBlock("Cold-root");
  expect(selectedIds()).toEqual(["Cold-root"]);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Other", pageKind: "page" });
  await vi.waitFor(() => expect(host.textContent).toContain("Other visible content"));
  expect(host.textContent).not.toContain("Cold visible content");
  expect(selectedIds()).toEqual([]);
});

it.each(["link", "palette", "back", "forward", "keyboard"])("Delete after %s navigation leaves the departed page unchanged", async entry => {
  const { host } = await mounted();
  if (entry === "back") mainPaneRouter.openPage("Other", "page", { inPlace: true });
  if (entry === "forward") {
    mainPaneRouter.openPage("Other", "page", { inPlace: true });
    mainPaneRouter.goBack();
  }
  const source = entry === "back" ? "Other" : "Cold";
  await vi.waitFor(() => expect(host.textContent).toContain(`${source} visible content`));
  selectBlock(`${source}-root`);
  const before = pageToDto(source);
  if (entry === "link") host.querySelector<HTMLAnchorElement>("a.page-ref")!.click();
  if (entry === "palette") {
    const paletteHost = document.createElement("div"); document.body.append(paletteHost);
    disposers.push(render(() => <QuickSwitcher />, paletteHost));
    setRecentPages([{ name: "Other", kind: "page", path: "pages/Other.md" }]);
    openSwitcher({ mode: "pages" });
    const row = await vi.waitFor(() => {
      const row = [...paletteHost.querySelectorAll<HTMLElement>(".switcher-row")].find(row => row.textContent?.includes("Other"));
      expect(row).toBeDefined(); return row!;
    });
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
  }
  if (entry === "back") mainPaneRouter.goBack();
  if (entry === "forward") mainPaneRouter.goForward();
  if (entry === "keyboard") runGlobalCommand("go/journals");
  await vi.waitFor(() => expect(host.textContent).not.toContain(`${source} visible content`));
  pressDelete();
  expect(pageToDto(source)).toEqual(before);
  expect(selectedIds()).toEqual([]);
  undo();
  expect(pageToDto(source)).toEqual(before);
});

it("retiring another pane preserves selection, while the selected right pane's route retires it", async () => {
  const right = splitPane("main", "row")!;
  const leftView = await mounted();
  const rightRouter = paneRouter(right);
  const rightView = await mounted(rightRouter);
  setFocusedPaneId(right); noteSurfaceFocused(`pane:${right}`);
  selectBlock("Cold-root");
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Other", pageKind: "page" });
  await vi.waitFor(() => expect(leftView.host.textContent).toContain("Other visible content"));
  expect(selectedIds()).toEqual(["Cold-root"]);
  leftView.dispose();
  expect(selectedIds()).toEqual(["Cold-root"]);
  const before = pageToDto("Cold");
  rightRouter.replaceActiveRoute({ kind: "page", name: "Other", pageKind: "page" });
  await vi.waitFor(() => expect(rightView.host.textContent).toContain("Other visible content"));
  pressDelete();
  expect(pageToDto("Cold")).toEqual(before);
  expect(selectedIds()).toEqual([]);
});

it("unmounting the selected surface retires selection", async () => {
  const { dispose } = await mounted();
  selectBlock("Cold-root");
  const before = pageToDto("Cold");
  dispose(); pressDelete();
  expect(pageToDto("Cold")).toEqual(before);
  expect(selectedIds()).toEqual([]);
});

it("a sidebar selection survives main-pane navigation and retires when its own card closes", async () => {
  const { host } = await mounted();
  applySidebarSession({ right: true, items: [{ kind: "page", name: "Cold", pageKind: "page" }] });
  const sidebar = document.createElement("div"); document.body.append(sidebar);
  disposers.push(render(() => <RightSidebar />, sidebar));
  await vi.waitFor(() => expect(sidebar.textContent).toContain("Cold visible content"));
  const surface = sidebar.querySelector<HTMLElement>("[data-sidebar-surface]")!.dataset.sidebarSurface!;
  noteSurfaceFocused(surface); selectBlock("Cold-root");
  mainPaneRouter.openPage("Other");
  await vi.waitFor(() => expect(host.textContent).toContain("Other visible content"));
  expect(selectedIds()).toEqual(["Cold-root"]);
  const before = pageToDto("Cold");
  setRightSidebar([]); pressDelete();
  expect(selectedIds()).toEqual([]);
  expect(pageToDto("Cold")).toEqual(before);
});

it("navigation retires selection made by Escape inside a nested editable page embed", async () => {
  vi.mocked(backend().getPage).mockImplementation(async name => name === "Cold"
    ? { ...page(name), blocks: [...page(name).blocks, { id: "embed", raw: "{{embed [[Source]]}}", children: [], collapsed: false }] }
    : page(name));
  const { host } = await mounted();
  const content = await vi.waitFor(() => {
    const content = host.querySelector<HTMLElement>('.embed-block [data-block-id="Source-root"] > .block-main .block-content');
    expect(content).not.toBeNull(); return content!;
  });
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  const editor = await vi.waitFor(() => {
    const editor = maybeEditorIn(host, "Source-root"); expect(editor).not.toBeNull(); return editor!;
  });
  editor.keydown({ key: "Escape", code: "Escape" });
  expect(selectedIds()).toEqual(["Source-root"]);
  const before = pageToDto("Source");
  mainPaneRouter.openPage("Other");
  await vi.waitFor(() => expect(host.textContent).toContain("Other visible content"));
  pressDelete();
  expect(selectedIds()).toEqual([]);
  expect(pageToDto("Source")).toEqual(before);
});

// Merge review F1 (delete) and round 2 (merge): the owner's own reselection after deleting a
// selected embed host, and after merging, keeps the selection's surface, not the unmounted
// inner editor's, so navigation still retires it and a later Delete cannot reach the departed page.
it.each(["delete", "merge"])("the block reselected after %s is still retired by navigation", async after => {
  vi.mocked(backend().getPage).mockImplementation(async name => name === "Cold"
    ? { ...page(name), blocks: [...page(name).blocks, { id: "embed", raw: "{{embed [[Source]]}}", children: [], collapsed: false }] }
    : page(name));
  const { host } = await mounted();
  const content = await vi.waitFor(() => {
    const content = host.querySelector<HTMLElement>('.embed-block [data-block-id="Source-root"] > .block-main .block-content');
    expect(content).not.toBeNull(); return content!;
  });
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  await vi.waitFor(() => expect(maybeEditorIn(host, "Source-root")).not.toBeNull());
  expect(dispatchFocusedEditorCommand("editor/select-embedding-block")).toBe(true);
  expect(selectedIds()).toEqual(["embed"]);
  pressDelete();
  expect(selectedIds()).toEqual(["Cold-link"]);
  if (after === "merge") {
    moveSelection(-1, true);
    expect(selectedIds()).toEqual(["Cold-root", "Cold-link"]);
    expect(mergeSelection()).toBe(true);
    expect(selectedIds()).toEqual(["Cold-root"]);
  }
  const before = pageToDto("Cold");
  mainPaneRouter.openPage("Other");
  await vi.waitFor(() => expect(host.textContent).toContain("Other visible content"));
  pressDelete();
  expect(selectedIds()).toEqual([]);
  expect(pageToDto("Cold")).toEqual(before);
});

it("paging and scrolling to another journal day preserves the still-visible selection", async () => {
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
  selectBlock(`${today.name}-root`);
  await extendFeedForScroll();
  host.dispatchEvent(new Event("scroll"));
  await vi.waitFor(() => expect(host.textContent).toContain(`${older.name} visible content`));
  expect(host.textContent).toContain(`${today.name} visible content`);
  expect(selectedIds()).toEqual([`${today.name}-root`]);
  vi.unstubAllGlobals();
});

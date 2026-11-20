import type { PageDto, PageRead } from "./types";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import * as tauriCore from "@tauri-apps/api/core";
import { setToasts, toasts } from "./toasts";
import { backend } from "./backend";
import * as deepLinkNavigation from "./deepLinkNavigation";
import { installMobileExternalLinkHandler } from "./App";
import { App } from "./App";
import { render } from "solid-js/web";
import { paneRouter, resetPaneLayoutToSingle, restorePaneLayout, splitPane } from "./panes";
import { clearSelection, markDirty, node as docNode, resetStore, selectedIds, setRaw } from "./document";
import { setBlockMoving } from "./document/edits/moves";
import { pageToDto } from "./document/convert";
import { type FeedPage, type Node as StoreNode } from "./document/model";
import { setDoc } from "./document/model";
import { isConflicted } from "./document";
import { pageInventoryRev, firstLoadDone, setFirstLoadDone } from "./graphSession";
import { bumpGraphEpoch } from "./graphSession";
import { applyGraphChange as handleGraphChange } from "./document";
import { bindTestHost, type TestHost } from "./document/host/wiring.test.support";
import { journalTitle, localDayKey } from "./journal";
import { openPageInSidebar, setRightSidebar, setRightSidebarOpen } from "./ui";

vi.mock("@tauri-apps/api/core", { spy: true });

function addAnchor(href: string): HTMLAnchorElement {
  const a = document.createElement("a");
  a.href = href;
  a.textContent = href;
  document.body.appendChild(a);
  return a;
}

function click(el: Element): MouseEvent {
  const event = new MouseEvent("click", { bubbles: true, cancelable: true });
  el.dispatchEvent(event);
  return event;
}

function selectAllFrom(at: HTMLElement, scroller: HTMLElement): boolean {
  scroller.focus();
  click(at);
  const event = new KeyboardEvent("keydown", { key: "a", ctrlKey: true, bubbles: true, cancelable: true });
  scroller.dispatchEvent(event);
  return event.defaultPrevented;
}

// The window's page-mail listener is registered on its first bind; App renders
// bind too, so bind through the helper first to let later tests deliver mail.
beforeAll(async () => { await bindTestHost(); });

it("Journals navigation uses the same glyph in the sidebar and topbar (GH #673)", () => {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <App />, host);
  try {
    const top = host.querySelector('button[title="Journals"] svg');
    const side = [...host.querySelectorAll(".left-sidebar .nav-item")].find(el => el.textContent === "Journals")?.querySelector("svg");
    expect(top).not.toBeNull();
    expect(side).toBeDefined();
    expect(side!.innerHTML).toBe(top!.innerHTML);
  } finally { dispose(); }
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
  resetStore();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
});

it("ends a routed page blank selection when keyboard focus moves to the sidebar", async () => {
  const [body, sidebar] = ["Body", "Sidebar"].map(name => ({
    name, title: name, kind: "page" as const, id: `pages/${name}.md`, pre_block: null,
    blocks: [{ id: name.toLowerCase(), raw: `${name} block`, collapsed: false, children: [] }],
  }));
  vi.spyOn(backend(), "getPage").mockImplementation(async name => [body, sidebar].find(page => page.name === name) ?? null);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => [body, sidebar].find(page => page.id === path) ?? null);
  const submit = vi.spyOn(backend(), "pageSubmit");
  setFirstLoadDone(false);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <App />, host);
  try {
    await vi.waitFor(() => expect(firstLoadDone()).toBe(true));
    paneRouter("main").replaceActiveRoute({ kind: "page", name: body.name, pageKind: "page" });
    openPageInSidebar(sidebar.name, sidebar.kind, sidebar.id);
    const find = (selector: string) => vi.waitFor(() => {
      const element = host.querySelector<HTMLElement>(selector);
      expect(element).not.toBeNull();
      return element!;
    });
    const row = await find('.main-content [data-block-id="body"] > .block-main');
    const scroller = await find(".main-content");
    const control = await find(".right-sidebar .rs-item-toggle");
    const sidebarRow = await find('.right-sidebar [data-block-id="sidebar"] > .block-main');
    expect(selectAllFrom(row, scroller)).toBe(true);
    expect(selectedIds()).toEqual(["body"]);
    control.focus();
    expect(document.activeElement).toBe(control);
    const afterFocus = selectedIds();
    const remove = new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true });
    control.dispatchEvent(remove);
    expect(afterFocus).toEqual([]);
    expect(remove.defaultPrevented).toBe(false);
    const field = sidebarRow.closest(".right-sidebar")!.appendChild(document.createElement("input"));
    const clipboard = vi.spyOn(backend(), "writeRich");
    for (const [at, focus] of [[sidebarRow, control], [field, field]]) {
      for (const key of ["Delete", "Backspace", "x"]) {
        expect(selectAllFrom(at, focus)).toBe(false);
        const event = new KeyboardEvent("keydown", { key, ctrlKey: key === "x", bubbles: true, cancelable: true });
        focus.dispatchEvent(event);
        expect(event.defaultPrevented).toBe(false);
        expect(selectedIds()).toEqual([]);
        expect(docNode("body")?.raw).toBe("Body block");
        expect(docNode("sidebar")?.raw).toBe("Sidebar block");
      }
    }
    expect(clipboard).not.toHaveBeenCalled();
    expect(submit).not.toHaveBeenCalled();

    scroller.focus();
    click(row);
    paneRouter("main").replaceActiveRoute({ kind: "page", name: sidebar.name, pageKind: "page" });
    await find('.main-content [data-block-id="sidebar"]');
    const select = new KeyboardEvent("keydown", { key: "a", ctrlKey: true, bubbles: true, cancelable: true });
    scroller.dispatchEvent(select);
    expect(select.defaultPrevented).toBe(false);
    expect(selectedIds()).toEqual([]);
  } finally {
    dispose();
    setRightSidebar([]);
    setRightSidebarOpen(false);
  }
});

it.each([false, true])("selects the routed page or zoom from scroller whitespace (split pane: %s)", async (split) => {
  const roots = [
    { id: "zoom-root", raw: "Zoom root", collapsed: true, children: [{ id: "zoom-child", raw: "Zoom child", collapsed: false, children: [] }] },
    { id: "page-sibling", raw: "Page sibling {{embed [[Embedded]]}}", collapsed: false, children: [] },
  ];
  const routed: PageRead = { name: "Outline", title: "Outline", kind: "page", id: "pages/Outline.md", pre_block: null, blocks: roots };
  const embedded: PageRead = { name: "Embedded", title: "Embedded", kind: "page", id: "pages/Embedded.md", pre_block: null,
    blocks: [{ id: "embedded-root", raw: "Embedded body", collapsed: false, children: [] }] };
  vi.spyOn(backend(), "getPage").mockImplementation(async name => name === routed.name ? routed : name === embedded.name ? embedded : null);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => path === routed.id ? routed : path === embedded.id ? embedded : null);
  setFirstLoadDone(false);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <App />, host);
  try {
    await vi.waitFor(() => expect(firstLoadDone()).toBe(true));
    const paneId = split ? splitPane("main", "row")! : "main";
    const router = paneRouter(paneId);
    router.replaceActiveRoute({ kind: "page", name: routed.name, pageKind: "page" });
    const scroller = await vi.waitFor(() => {
      const el = host.querySelector<HTMLElement>(split ? `[data-pane-id="${paneId}"] .main-content` : ".main-content");
      expect(el?.textContent).toContain("Page sibling");
      return el!;
    });
    for (const zone of [scroller, scroller.querySelector<HTMLElement>(".main-content-inner")!]) {
      expect(selectAllFrom(zone, scroller)).toBe(true);
      expect(selectedIds()).toEqual(["zoom-root", "zoom-child", "page-sibling"]);
      clearSelection();
    }
    const embed = await vi.waitFor(() => {
      const el = scroller.querySelector<HTMLElement>(".embed-block");
      expect(el?.textContent).toContain("Embedded body");
      return el!;
    });
    expect(selectAllFrom(embed, scroller)).toBe(true);
    expect(selectedIds()).toEqual(["embedded-root"]);
    router.zoomToBlock({ name: routed.name, pageKind: "page", block: "zoom-root" });
    await vi.waitFor(() => expect(scroller.querySelector(".zoomed-page")?.textContent).toContain("Zoom child"));
    for (const zone of [scroller, scroller.querySelector<HTMLElement>(".main-content-inner")!]) {
      expect(selectAllFrom(zone, scroller)).toBe(true);
      expect(selectedIds()).toEqual(["zoom-root", "zoom-child"]);
      clearSelection();
    }
  } finally { dispose(); }
});

it("keeps Journals day scopes after a routed page leaves the scroller", async () => {
  const today = new Date();
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  const days = [today, yesterday].map((date, index): PageRead => ({
    name: journalTitle(date), title: journalTitle(date), kind: "journal", id: `journals/${localDayKey(date)}.md`, pre_block: null,
    blocks: [{ id: `day-${index}`, raw: `Journal body ${index}`, collapsed: false, children: [] }],
  }));
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages: days, next_before_day: null, done: true, as_of_day: localDayKey(today) });
  const routed: PageRead = { name: "Outline", title: "Outline", kind: "page", id: "pages/Outline.md", pre_block: null,
    blocks: [{ id: "page-root", raw: "Routed body", collapsed: false, children: [] }] };
  vi.spyOn(backend(), "getPage").mockResolvedValue(routed);
  const router = paneRouter("main");
  router.replaceActiveRoute({ kind: "page", name: routed.name, pageKind: "page" });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <App />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".main-content")?.textContent).toContain("Routed body"));
    const scroller = host.querySelector<HTMLElement>(".main-content")!;
    router.replaceActiveRoute({ kind: "journals" });
    await vi.waitFor(() => expect(scroller.textContent).toContain("Journal body 1"));
    expect(selectAllFrom(scroller, scroller)).toBe(false);
    for (const [index, day] of days.entries()) {
      expect(selectAllFrom(scroller.querySelector<HTMLElement>(`[data-page-name="${day.name}"] .page-blocks`)!, scroller)).toBe(true);
      expect(selectedIds()).toEqual([`day-${index}`]);
    }
  } finally { dispose(); }
});

it.each(["zoom", "Journals day"].flatMap(surface =>
  [".block-main", ".block-children"].map(zone => ({ surface, zone }))
))("row padding and child indentation restart selection in $surface ($zone)", async ({ surface, zone }) => {
  const today = new Date();
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  const outline = (id: string) => ({
    id, raw: `Body ${id}`, collapsed: false,
    children: [{ id: `${id}-child`, raw: `Child of ${id}`, collapsed: false, children: [] }],
  });
  const days = [today, yesterday].map((date, index): PageRead => ({
    name: journalTitle(date), title: journalTitle(date), kind: "journal", id: `journals/${localDayKey(date)}.md`, pre_block: null,
    blocks: [outline(`day-${index}`)],
  }));
  const routed: PageRead = {
    name: "Routed", title: "Routed", kind: "page", id: "pages/Routed.md", pre_block: null,
    blocks: [outline("routed-root")],
  };
  if (surface === "zoom") routed.blocks[0].children.push({ id: "embed-host", raw: "{{embed [[Other]]}}", collapsed: false, children: [] });
  const other: PageRead = { name: "Other", title: "Other", kind: "page", id: "pages/Other.md", pre_block: null, blocks: [outline("other-root")] };
  vi.spyOn(backend(), "getPage").mockImplementation(async name => [routed, other].find(page => page.name === name) ?? null);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => [routed, other].find(page => page.id === path) ?? null);
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages: days, next_before_day: null, done: true, as_of_day: localDayKey(today) });
  const router = paneRouter("main");
  const journal = surface === "Journals day";
  setFirstLoadDone(false);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <App />, host);
  try {
    await vi.waitFor(() => expect(firstLoadDone()).toBe(true));
    router.replaceActiveRoute(journal ? { kind: "journals" } : { kind: "page", name: routed.name, pageKind: "page" });
    const find = async (selector: string) => vi.waitFor(() => {
      const element = host.querySelector<HTMLElement>(selector);
      expect(element).not.toBeNull();
      return element!;
    });
    const routedBody = `.main-content [data-page-name="${routed.name}"] .page-blocks`;
    if (!journal) await find(routedBody);
    if (surface === "zoom") router.zoomToBlock({ name: routed.name, pageKind: "page", block: "routed-root" });
    const from = await find(journal ? `[data-page-name="${days[0].name}"] .page-blocks`
      : ".zoomed-page .embed-block");
    const id = journal ? "day-1" : "routed-root";
    const scope = journal ? ".main-content" : ".zoomed-page";
    const to = await find(`${scope} [data-block-id="${id}"] ${zone}`);
    expect(selectAllFrom(from, host.querySelector<HTMLElement>(".main-content")!)).toBe(true);
    expect(selectedIds()).toContain(journal ? "day-0" : "other-root");
    to.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
    to.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, button: 0 }));
    to.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
    expect(selectedIds()).toEqual([]);
    const event = new KeyboardEvent("keydown", { key: "a", ctrlKey: true, bubbles: true, cancelable: true });
    const home = to.closest<HTMLElement>(".main-content")!;
    home.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    expect(selectedIds()).toEqual([id, `${id}-child`, ...(surface === "zoom" ? ["embed-host"] : [])]);
  } finally {
    dispose();
    setRightSidebar([]);
    setRightSidebarOpen(false);
  }
});

function page(name: string, kind: "page" | "journal", roots: string[]): FeedPage {
  return { name, kind, title: name, preBlock: null, roots, format: "md", readOnly: false, guide: false };
}

/** Mark `name` dirty on a bound host: the noted edit opens the page there, so
 * the host holds it. Returns the host and the page's key. */
async function heldDirty(name: string): Promise<{ host: TestHost; key: string }> {
  const host = await bindTestHost();
  const open = vi.spyOn(backend(), "pageOpen");
  markDirty(name, "save-block");
  await vi.waitFor(() => expect(open).toHaveBeenCalledTimes(1));
  const opened = await open.mock.results[0].value;
  if (!opened || !("key" in opened)) throw new Error(`the host refused to open ${name}`);
  return { host, key: opened.key };
}

function node(id: string, pageName: string): StoreNode {
  return { id, raw: "loaded elsewhere", collapsed: false, parent: null, page: pageName, children: [] };
}

describe("mobile external link delegation", () => {
  it.each(["onGraphChanged", "onConflictsChanged"] as const)("keeps %s registered when graph B opens before registration returns", async (method) => {
    let finish!: (unlisten: () => void) => void;
    const unlisten = vi.fn();
    vi.spyOn(backend(), method).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <App />, host);
    try {
      await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
      bumpGraphEpoch();
      finish(unlisten);
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(unlisten).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
    await vi.waitFor(() => expect(unlisten).toHaveBeenCalledOnce());
  });
  it("releases a watcher handle that arrives after App unmounts", async () => {
    let finish!: (unlisten: () => void) => void;
    const unlisten = vi.fn();
    vi.spyOn(backend(), "onConflictsChanged").mockImplementationOnce(() =>
      new Promise((resolve) => { finish = resolve; }));
    vi.spyOn(backend(), "onGraphChanged").mockResolvedValue(() => {});
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <App />, host);
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    dispose();
    finish(unlisten);
    await vi.waitFor(() => expect(unlisten).toHaveBeenCalledOnce());
  });
  it("does not install a link listener after its view retires during platform lookup", async () => {
    let finish!: (platform: "android") => void;
    vi.spyOn(backend(), "appPlatform").mockImplementationOnce(() =>
      new Promise((resolve) => { finish = resolve; })
    );
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    let live = true;
    const pending = installMobileExternalLinkHandler(() => live);
    live = false;
    finish("android");
    const uninstall = await pending;
    try {
      expect(click(addAnchor("https://x.test/path")).defaultPrevented).toBe(false);
      expect(openExternal).not.toHaveBeenCalled();
    } finally { uninstall(); }
  });

  it("opens external links through the OS browser on Android", async () => {
    vi.spyOn(backend(), "appPlatform").mockResolvedValue("android");
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const uninstall = await installMobileExternalLinkHandler();
    try {
      const a = addAnchor("https://x.test/path");
      const targetClick = vi.fn();
      a.addEventListener("click", targetClick);

      const event = click(a);

      expect(event.defaultPrevented).toBe(true);
      expect(targetClick).not.toHaveBeenCalled();
      expect(openExternal).toHaveBeenCalledTimes(1);
      expect(openExternal).toHaveBeenCalledWith("https://x.test/path");
    } finally {
      uninstall();
    }
  });

  it("does not intercept external links on desktop", async () => {
    vi.spyOn(backend(), "appPlatform").mockResolvedValue("desktop");
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const uninstall = await installMobileExternalLinkHandler();
    try {
      const a = addAnchor("https://x.test/path");
      a.target = "_blank";
      const event = click(a);

      expect(event.defaultPrevented).toBe(false);
      expect(openExternal).not.toHaveBeenCalled();
    } finally {
      uninstall();
    }
  });

  it("I-22: blocks WebView navigation for any other explicit scheme on Android and keeps component handlers", async () => {
    vi.spyOn(backend(), "appPlatform").mockResolvedValue("android");
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const uninstall = await installMobileExternalLinkHandler();
    try {
      for (const href of ["javascript:alert(1)", "intent://x#Intent;end", "file:///sdcard/a.pdf"]) {
        const a = addAnchor(href);
        const own = vi.fn();
        a.addEventListener("click", own);
        expect(click(a).defaultPrevented, href).toBe(true);
        expect(own, href).toHaveBeenCalledOnce();
      }
      expect(openExternal).not.toHaveBeenCalled();
    } finally {
      uninstall();
    }
  });

  it("leaves internal hash links untouched on Android", async () => {
    vi.spyOn(backend(), "appPlatform").mockResolvedValue("android");
    const openExternal = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    const uninstall = await installMobileExternalLinkHandler();
    try {
      const event = click(addAnchor("#x"));

      expect(event.defaultPrevented).toBe(false);
      expect(openExternal).not.toHaveBeenCalled();
    } finally {
      uninstall();
    }
  });
});

describe("journal watcher feed reconciliation", () => {
  it("rekeys an externally edited title by physical path before publishing the new page", async () => {
    const id = "pages/Physical.md";
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Physical", pageKind: "page", path: id }], pos: 0, pinned: false }], activeIndex: 0 });
    setDoc({ byId: { body: node("body", "Physical") },
      pages: [{ ...page("Physical", "page", ["body"]), id }], feed: ["Physical"], loaded: true });
    const dto: PageRead = { name: "Effective", kind: "page", title: "Effective", id, rev: "new-rev",
      pre_block: "title:: Effective", blocks: [{ id: "body", raw: "external", children: [], collapsed: false }] };
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    await handleGraphChange({ path: id, name: "Effective", kind: "page", created: false, removed: false });
    expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Effective", path: id });
    expect(pageToDto("Effective")?.blocks[0].raw).toBe("external");
    expect(pageToDto("Physical")).toBeNull();
  });
  for (const branch of ["open page", "feed day", "loaded satellite"] as const) {
    it(`keeps an edit typed during the ${branch} watcher fetch`, async () => {
      const name = branch === "feed day" ? "15th July, 2030" : `Watcher ${branch}`;
      const kind = branch === "feed day" ? "journal" : "page";
      resetPaneLayoutToSingle({ tabs: [{ history: [branch === "open page"
        ? { kind: "page", name, pageKind: kind } as const
        : { kind: "journals" } as const], pos: 0, pinned: false }], activeIndex: 0 });
      setDoc({ byId: { local: { ...node("local", name), raw: "before" } }, pages: [page(name, kind, ["local"])], feed: branch === "feed day" ? [name] : [], loaded: true });
      let finish!: (dto: PageRead | null) => void;
      const read = vi.spyOn(backend(), "getPage").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
      const changed = handleGraphChange({ name, kind, created: false, removed: false });
      await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
      setRaw("local", "typed during fetch", { timetracking: false });
      finish({ name, kind, title: name, id: `${kind}s/${name}.md`, pre_block: null,
        blocks: [{ id: "disk", raw: "from disk", collapsed: false, children: [] }] });
      await changed;
      expect(pageToDto(name)!.blocks[0].raw).toBe("typed during fetch");
    });
  }
  it("keeps a clean page loaded when its own saved bytes echo through the watcher", async () => {
    const name = "Own save";
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    setDoc({ byId: { same: { ...node("same", name), raw: "saved" } }, pages: [page(name, "page", ["same"])], feed: [name], loaded: true });
    const read = vi.spyOn(backend(), "getPage").mockResolvedValue({ name, kind: "page", title: name, id: "pages/Own save.md", pre_block: null, blocks: [{ id: "same", raw: "saved", collapsed: false, children: [] }] } as PageRead);

    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(read).toHaveBeenCalledTimes(1);
    expect(pageToDto(name)!.blocks[0].raw).toBe("saved");
  });

  // A page the host holds takes an external change as host mail: the watcher
  // event neither reloads nor reads it, and the host's conflict (over the
  // window's input) is what marks it conflicted.
  it("marks a dirty page conflicted and preserves its edits after an external change", async () => {
    const name = "Dirty";
    setDoc({ byId: { local: { ...node("local", name), raw: "local edit" } }, pages: [page(name, "page", ["local"])], feed: [], loaded: true });
    const { host, key } = await heldDirty(name);
    const read = vi.spyOn(backend(), "getPage");

    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(read).not.toHaveBeenCalled();
    expect(pageToDto(name)!.blocks[0].raw).toBe("local edit");
    expect(isConflicted(name)).toBe(false);
    host.notice(key, { conflictReported: true }, { version: 9, conflict: true, disk: { kind: "file", rev: "external-rev" } });
    await vi.waitFor(() => expect(isConflicted(name)).toBe(true));
    expect(pageToDto(name)!.blocks[0].raw).toBe("local edit");
  });

  it("keeps a removed dirty page open as a conflict", async () => {
    const name = "Removed while dirty";
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    setDoc({ byId: { local: node("local", name) }, pages: [page(name, "page", ["local"])], feed: [name], loaded: true });
    const { host, key } = await heldDirty(name);
    await handleGraphChange({ name, kind: "page", created: false, removed: true });
    host.notice(key, { conflictReported: true }, { version: 9, conflict: true, disk: { kind: "no-file" } });
    await vi.waitFor(() => expect(isConflicted(name)).toBe(true));
    expect(paneRouter("main").route()).toMatchObject({ kind: "page", name });
  });

  it("leaves a page alone while a block move is in progress", async () => {
    const name = "Moving";
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    setDoc({ byId: { old: node("old", name) }, pages: [page(name, "page", ["old"])], feed: [name], loaded: true });
    const read = vi.spyOn(backend(), "getPage");
    setBlockMoving(true);
    try {
      await handleGraphChange({ name, kind: "page", created: false, removed: false });
      expect(read).not.toHaveBeenCalled();
      expect(isConflicted(name)).toBe(false);
    } finally {
      setBlockMoving(false);
    }
  });

  it("reloads a loaded satellite page outside the visible route and feed", async () => {
    const name = "Satellite";
    setDoc({ byId: { old: node("old", name) }, pages: [page(name, "page", ["old"])], feed: [], loaded: true });
    vi.spyOn(backend(), "getPage").mockResolvedValue({ name, kind: "page", title: name, id: "pages/Satellite.md", pre_block: null, blocks: [{ id: "fresh", raw: "external", collapsed: false, children: [] }] } as PageRead);
    await handleGraphChange({ name, kind: "page", created: false, removed: false });
    expect(pageToDto(name)!.blocks[0].raw).toBe("external");
  });

  it("restarts Journals for an unloaded journal without fetching a page DTO", async () => {
    const name = "15th July, 2030";
    const read = vi.spyOn(backend(), "getPage");
    const now = new Date();
    const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true,
      as_of_day: now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate(),
    });
    await handleGraphChange({ name, kind: "journal", created: false, removed: false });
    await Promise.resolve();
    expect(read).not.toHaveBeenCalled();
    expect(feed).toHaveBeenCalledWith(3, null);
  });

  it("reloads a loaded journal in Journals and restarts its feed", async () => {
    const name = "15th July, 2030";
    setDoc({ byId: { old: node("old", name) }, pages: [page(name, "journal", ["old"])], feed: [], loaded: true });
    const read = vi.spyOn(backend(), "getPage").mockResolvedValue({ name, kind: "journal", title: name, id: "journals/2030_07_15.md", pre_block: null, blocks: [{ id: "fresh", raw: "external journal", collapsed: false, children: [] }] } as PageRead);
    const now = new Date();
    const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true,
      as_of_day: now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate(),
    });
    await handleGraphChange({ name, kind: "journal", created: false, removed: false });
    await Promise.resolve();
    expect(read).toHaveBeenCalledTimes(1);
    expect(pageToDto(name)!.blocks[0].raw).toBe("external journal");
    expect(feed).toHaveBeenCalledWith(3, null);
  });

  it("restarts Journals while preserving a dirty journal removed on disk", async () => {
    const name = "15th July, 2030";
    setDoc({ byId: { local: node("local", name) }, pages: [page(name, "journal", ["local"])], feed: [], loaded: true });
    const { host, key } = await heldDirty(name);
    const now = new Date();
    const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true,
      as_of_day: now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate(),
    });
    await handleGraphChange({ name, kind: "journal", created: false, removed: true });
    await Promise.resolve();
    host.notice(key, { conflictReported: true }, { version: 9, conflict: true, disk: { kind: "no-file" } });
    await vi.waitFor(() => expect(isConflicted(name)).toBe(true));
    expect(pageToDto(name)!.blocks[0].raw).toBe("loaded elsewhere");
    expect(feed).toHaveBeenCalledWith(3, null);
  });

  it("navigates away from a removed page", async () => {
    const name = "Deleted";
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    await handleGraphChange({ name, kind: "page", created: false, removed: true });
    expect(paneRouter("main").route().kind).toBe("journals");
  });

  it("does not mark a new graph page conflicted from an old watcher event (I-20)", async () => {
    const name = "Same name";
    setDoc({ byId: { fresh: node("fresh", name) }, pages: [page(name, "page", ["fresh"])], feed: [], loaded: true });
    markDirty(name, "save-block");
    await handleGraphChange({ name, kind: "page", created: false, removed: true, binding_generation: Number.MAX_SAFE_INTEGER });
    expect(isConflicted(name), "I-20: old watcher events must not block the new graph; exemplar src/App.tsx handleGraphChange").toBe(false);
  });

  it("does not reload old watcher bytes into a same-name page after a graph switch (I-20)", async () => {
    const name = "Same name";
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    setDoc({ byId: { old: { ...node("old", name), raw: "old graph" } }, pages: [page(name, "page", ["old"])], feed: [name], loaded: true });
    let finish!: (dto: PageRead | null) => void;
    const read = vi.spyOn(backend(), "getPage").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const changed = handleGraphChange({ name, kind: "page", created: false, removed: false });
    await vi.waitFor(() => expect(read).toHaveBeenCalled());
    resetStore();
    setDoc({ byId: { fresh: { ...node("fresh", name), raw: "new graph" } }, pages: [page(name, "page", ["fresh"])], feed: [name], loaded: true });
    finish({ name, kind: "page", title: name, id: "pages/Same name.md", pre_block: null, blocks: [{ id: "stale", raw: "stale watcher", collapsed: false, children: [] }] });
    await changed;
    expect(pageToDto(name)!.blocks[0].raw).toBe("new graph");
  });

  it("restarts a live Journals feed when the changed journal was already loaded in another pane", async () => {
    const name = "15th July, 2030";
    restorePaneLayout(
      { kind: "split", dir: "row", ratio: 0.5, children: [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: "pane-2" }] },
      new Map([
        ["main", { tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 }],
        ["pane-2", { tabs: [{ history: [{ kind: "page", name, pageKind: "journal" }], pos: 0, pinned: false }], activeIndex: 0 }],
      ]),
      "main"
    );
    setDoc({ byId: { loaded: node("loaded", name) }, pages: [page(name, "journal", ["loaded"])], feed: [], loaded: true });
    vi.spyOn(backend(), "getPage").mockResolvedValue({ name, kind: "journal", title: name, pre_block: null, blocks: [] } as PageDto as PageRead);
    const now = new Date();
    const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true,
      as_of_day: now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate(),
    });

    await handleGraphChange({ name, kind: "journal", created: false, removed: false });
    await Promise.resolve();
    expect(feed).toHaveBeenCalledTimes(1);
    expect(feed).toHaveBeenCalledWith(3, null);
  });
});

describe("watcher page inventory invalidation", () => {
  it("bumps the rare page-inventory revision for an external create", async () => {
    const before = pageInventoryRev();
    await handleGraphChange({ name: "Created Elsewhere", kind: "page", created: true, removed: false });
    expect(pageInventoryRev()).toBeGreaterThan(before);
  });
});

describe("identifier migration notice", () => {
  it("does not show a late migration toast after App unmounts", async () => {
    setToasts([]);
    let finish!: (value: boolean) => void;
    const take = vi.spyOn(tauriCore, "invoke").mockImplementationOnce(() =>
      new Promise((resolve) => { finish = resolve; }));
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <App />, host);
    await vi.dynamicImportSettled();
    await vi.waitFor(() => expect(take).toHaveBeenCalledWith("take_identifier_migration_notice"));
    dispose();
    finish(true);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(toasts().some((toast) => toast.message.includes("moved your settings and backups"))).toBe(false);
    setToasts([]);
  });
  it("shows the native one-shot notice stickily once, then stays silent on remount", async () => {
    setToasts([]);
    const take = vi.spyOn(tauriCore, "invoke").mockResolvedValue(false);
    take.mockResolvedValueOnce(true);
    const host = document.createElement("div");
    document.body.append(host);
    let dispose = render(() => <App />, host);
    try {
      await vi.dynamicImportSettled();
      await vi.waitFor(() => expect(toasts().some((toast) => toast.message.includes("moved your settings and backups"))).toBe(true));
      const shown = toasts().filter((toast) => toast.message.includes("moved your settings and backups"));
      expect(shown).toHaveLength(1);
      expect(shown[0].sticky).toBe(true);
      dispose();
      dispose = render(() => <App />, host);
      await vi.dynamicImportSettled();
      await vi.waitFor(() => expect(take).toHaveBeenCalledTimes(2));
      expect(toasts().filter((toast) => toast.message.includes("moved your settings and backups"))).toHaveLength(1);
    } finally { dispose(); setToasts([]); }
  });
});

describe("first load completion", () => {
  it("loads the remembered graph on a link launch before installing the link listener", async () => {
    setFirstLoadDone(false);
    vi.spyOn(backend(), "startupGraphPath").mockResolvedValue("/tmp/link-last-graph");
    const load = vi.spyOn(backend(), "loadGraph").mockRejectedValue(new Error("probe reached graph load"));
    const install = vi.spyOn(deepLinkNavigation, "installTineLinks").mockResolvedValue(() => {});
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <App />, host);
    try {
      await vi.waitFor(() => expect(firstLoadDone()).toBe(true));
      expect(load).toHaveBeenCalledWith("/tmp/link-last-graph");
      await vi.waitFor(() => expect(install).toHaveBeenCalled());
      expect(load.mock.invocationCallOrder[0]).toBeLessThan(install.mock.invocationCallOrder[0]);
    } finally {
      dispose();
    }
  });
  it("I-20: marks the first load done after the startup graph load retires its own binding", async () => {
    // Opening a graph bumps the graph epoch, so an owner captured BEFORE the
    // load is always stale afterwards. Welcome's `mandatory` gate reads this
    // flag, so a flag gated on that owner never lets a failed first open reach
    // onboarding. The view (not the graph binding) owns this completion.
    setFirstLoadDone(false);
    vi.spyOn(backend(), "startupGraphPath").mockResolvedValue("/tmp/never-opened");
    vi.spyOn(backend(), "loadGraph").mockImplementation(async () => { bumpGraphEpoch(); throw new Error("no such graph"); });
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <App />, host);
    try {
      await vi.waitFor(() => expect(firstLoadDone()).toBe(true));
    } finally {
      dispose();
    }
  });
  it("does not mark the first load done after App unmounts mid-load", async () => {
    setFirstLoadDone(false);
    let release!: (path: string) => void;
    vi.spyOn(backend(), "startupGraphPath").mockImplementation(() => new Promise((resolve) => { release = resolve; }));
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <App />, host);
    await vi.waitFor(() => expect(release).toBeTypeOf("function"));
    dispose();
    release("");
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(firstLoadDone()).toBe(false);
  });
});

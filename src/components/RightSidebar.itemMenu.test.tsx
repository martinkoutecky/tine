// GH #462: each right-sidebar item has a "more actions" menu with OG's entries
// (og-1.0.0 components/right_sidebar.cljs `x-menu-content`).
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { endEdit } from "../editorController";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import type { PageDto } from "../types";
import { applySidebarSession, rightSidebar, setRightSidebar } from "../ui";
import { RightSidebar } from "./RightSidebar";
import { route } from "../router";

const page: PageDto = {
  name: "Item menu test",
  kind: "page",
  title: "Item menu test",
  pre_block: null,
  blocks: [
    { id: "im-one", raw: "One", collapsed: false, children: [] },
    { id: "im-two", raw: "Two", collapsed: false, children: [] },
  ],
};

beforeAll(async () => { await initParser(); });
afterEach(() => {
  endEdit("page-navigation");
  setRightSidebar([]);
  applySidebarSession({ right: false, items: [] });
  resetStore();
  document.body.innerHTML = "";
  localStorage.clear();
  vi.restoreAllMocks();
});

function mount() {
  loadSingle(page);
  applySidebarSession({
    right: true,
    items: [
      { kind: "page", name: page.name, pageKind: "page" },
      { kind: "block", uuid: "im-one", page: page.name, pageKind: "page" },
      { kind: "block", uuid: "im-two", page: page.name, pageKind: "page" },
    ],
  });
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(() => <RightSidebar />, root) };
}

function openMenu(root: HTMLElement, index: number) {
  root.querySelectorAll<HTMLButtonElement>("[data-right-sidebar-item-actions]")[index].click();
}
function choose(action: string) {
  document.querySelector<HTMLButtonElement>(`[data-right-sidebar-item-action="${action}"]`)!.click();
}
const names = () => rightSidebar().map((item) => (item.kind === "page" ? `page:${item.name}` : `block:${item.uuid}`));
const collapsed = () => rightSidebar().map((item) => !!item.collapsed);

describe("right sidebar per-item menu (GH #462)", () => {
  it("Close others keeps only this item, expanded", () => {
    const { root, dispose } = mount();
    try {
      setRightSidebar(rightSidebar().map((item, i) => (i === 1 ? { ...item, collapsed: true } : item)));
      openMenu(root, 1);
      choose("close-others");
      expect(names()).toEqual(["block:im-one"]);
      expect(collapsed()).toEqual([false]);
      expect(document.querySelector("[data-right-sidebar-item-action]")).toBeNull();
    } finally { dispose(); }
  });

  it("Collapse others collapses every other item and leaves this one as it was", () => {
    const { root, dispose } = mount();
    try {
      openMenu(root, 2);
      choose("collapse-others");
      expect(names()).toHaveLength(3);
      expect(collapsed()).toEqual([true, true, false]);
    } finally { dispose(); }
  });

  it("Close closes this item; Close all empties the sidebar", () => {
    const { root, dispose } = mount();
    try {
      openMenu(root, 0);
      choose("close");
      expect(names()).toEqual(["block:im-one", "block:im-two"]);
      openMenu(root, 0);
      choose("close-all");
      expect(rightSidebar()).toEqual([]);
    } finally { dispose(); }
  });

  it("offers Collapse on an open item and Expand on a collapsed one; Collapse all and Expand all act on every item", () => {
    const { root, dispose } = mount();
    try {
      openMenu(root, 1);
      expect(document.querySelector('[data-right-sidebar-item-action="expand"]')).toBeNull();
      choose("collapse");
      expect(collapsed()).toEqual([false, true, false]);
      openMenu(root, 1);
      expect(document.querySelector('[data-right-sidebar-item-action="collapse"]')).toBeNull();
      choose("expand");
      expect(collapsed()).toEqual([false, false, false]);
      openMenu(root, 0);
      choose("collapse-all");
      expect(collapsed()).toEqual([true, true, true]);
      openMenu(root, 0);
      choose("expand-all");
      expect(collapsed()).toEqual([false, false, false]);
    } finally { dispose(); }
  });

  it("a lone item offers no others/all entries, and only a page item offers Open as page", () => {
    const { root, dispose } = mount();
    try {
      setRightSidebar([{ kind: "page", name: page.name, pageKind: "page" }]);
      openMenu(root, 0);
      const entries = [...document.querySelectorAll("[data-right-sidebar-item-action]")].map((el) => el.getAttribute("data-right-sidebar-item-action"));
      expect(entries).toEqual(["close", "collapse", "open-as-page"]);
      choose("open-as-page");
      expect(route()).toMatchObject({ kind: "page", name: page.name });
      setRightSidebar([{ kind: "block", uuid: "im-one", page: page.name, pageKind: "page" }]);
      openMenu(root, 0);
      expect(document.querySelector('[data-right-sidebar-item-action="open-as-page"]')).toBeNull();
    } finally { dispose(); }
  });
});

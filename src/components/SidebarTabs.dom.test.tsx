import { afterEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { paneRouter, resetPaneLayoutToSingle } from "../panes";
import { resetStore } from "../document";
import type { PaneSnapshot } from "../router";
import { SidebarTabs } from "./SidebarTabs";

// GH #704 v1 / D-20261018-W1: the open tabs as a sidebar list at every width
// (list, active highlight, switch, close). The component asks nothing about the
// device, so these cases hold on a phone exactly as on a desktop.

const snap = (names: string[], activeIndex = 0): PaneSnapshot => ({
  tabs: names.map((name) => ({ history: [{ kind: "page" as const, name, pageKind: "page" as const }], pos: 0, pinned: false })),
  activeIndex,
});

function mount(names: string[], activeIndex = 0) {
  resetPaneLayoutToSingle(snap(names, activeIndex));
  const host = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <SidebarTabs />, host);
  return { host, dispose };
}

const rows = (host: HTMLElement) => [...host.querySelectorAll<HTMLElement>(".nav-tab")];

afterEach(() => {
  resetStore();
  resetPaneLayoutToSingle(snap(["Home"]));
  document.body.replaceChildren();
});

describe("SidebarTabs (GH #704 v1)", () => {
  it("renders nothing for a single tab", () => {
    const { host, dispose } = mount(["Only"]);
    try {
      expect(host.querySelector('[data-sidebar-section="tabs"]')).toBeNull();
    } finally { dispose(); }
  });

  it("lists every tab with its title and highlights the active one", () => {
    const { host, dispose } = mount(["Alpha", "Beta", "Gamma"], 1);
    try {
      expect(rows(host).map((r) => r.querySelector(".nav-tab-title")?.textContent)).toEqual(["Alpha", "Beta", "Gamma"]);
      expect(rows(host).map((r) => r.classList.contains("active"))).toEqual([false, true, false]);
      expect(host.querySelector(".nav-section-count")?.textContent).toBe("3");
    } finally { dispose(); }
  });

  it("clicking a row switches the active tab and highlights it", () => {
    const { host, dispose } = mount(["Alpha", "Beta"], 0);
    try {
      rows(host)[1]!.querySelector<HTMLElement>(".nav-tab-main")!.click();
      const router = paneRouter("main");
      expect(router.activeId()).toBe(router.tabs()[1]!.id);
      expect(rows(host).map((r) => r.classList.contains("active"))).toEqual([false, true]);
    } finally { dispose(); }
  });

  it("the close button removes a tab and the section hides at one tab", async () => {
    const { host, dispose } = mount(["Alpha", "Beta"], 0);
    try {
      rows(host)[1]!.querySelector<HTMLElement>(".nav-tab-close")!.click();
      await Promise.resolve();
      expect(paneRouter("main").tabs()).toHaveLength(1);
      expect(host.querySelector('[data-sidebar-section="tabs"]')).toBeNull();
    } finally { dispose(); }
  });

  it("middle-click closes a tab; the header toggle collapses the list", async () => {
    const { host, dispose } = mount(["Alpha", "Beta", "Gamma"], 0);
    try {
      rows(host)[2]!.dispatchEvent(new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
      await Promise.resolve();
      expect(paneRouter("main").tabs()).toHaveLength(2);
      host.querySelector<HTMLElement>("[data-sidebar-section-toggle]")!.click();
      expect(rows(host)).toHaveLength(0);
      host.querySelector<HTMLElement>("[data-sidebar-section-toggle]")!.click();
      expect(rows(host)).toHaveLength(2);
    } finally { dispose(); }
  });
});

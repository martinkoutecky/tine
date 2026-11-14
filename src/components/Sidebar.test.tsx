import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { applySidebarSession, rightSidebar, setFavorites, setRecentPages } from "../ui";
import { setToasts, toasts, toUiText } from "../toasts";
import { renderEnglish } from "../i18n";
import { GraphSwitcher, Sidebar } from "./Sidebar";
import { mainPaneRouter, openJournals, openPageInNewTab, resetTabsToJournals, route } from "../router";
import { appNow, journalTitle } from "../journal";
import { runGlobalCommand } from "../keybindings";
import { readFileSync } from "node:fs";

it("GH #926: the named close icon shares the graph picker row and first-level rows share a casing class", () => {
  setFavorites([{ name: "Tine", kind: "page" }]);
  setRecentPages([{ name: "Tine", kind: "page" }]);
  mainPaneRouter.openPage("Project", "page", { inPlace: false });
  openPageInNewTab("Second project", "page");
  const close = vi.fn();
  const host = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <Sidebar onClose={close} />, host);
  try {
    const button = host.querySelector<HTMLButtonElement>('button[aria-label="Close navigation sidebar"]');
    expect(button).not.toBeNull();
    expect(button!.textContent).toBe("");
    expect(button!.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
    expect(button!.parentElement).toBe(host.querySelector(".graph-switch")!.parentElement);
    button!.click();
    expect(close).toHaveBeenCalledOnce();
    const rows = [...host.querySelectorAll(".nav-section-header, .nav-contents > .nav-item")];
    expect(rows.length).toBeGreaterThanOrEqual(5);
    for (const row of rows) expect(row.classList.contains("nav-first-level")).toBe(true);
    expect(host.textContent).toContain("Favorites");
    expect(host.textContent).toContain("Recent");
  } finally { dispose(); }
});

it("GH #740: Shift-click the top-left Journals entry keeps the main page and opens today in the right sidebar", () => {
  mainPaneRouter.openPage("Project", "page", { inPlace: true });
  const before = route();
  const completed = vi.fn();
  const root = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <Sidebar onActiveNavigationComplete={completed} />, root);
  try {
    const journals = [...root.querySelectorAll<HTMLElement>(".nav-item")].find(el => el.textContent === "Journals")!;
    journals.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
    expect(rightSidebar()).toContainEqual(expect.objectContaining({ kind: "page", name: journalTitle(appNow()), pageKind: "journal" }));
    expect(route()).toEqual(before);
    expect(completed).not.toHaveBeenCalled();

    // A plain click/tap still completes active navigation (closing the mobile drawer).
    journals.click();
    expect(route()).toMatchObject({ kind: "journals" });
    expect(completed).toHaveBeenCalledOnce();

    mainPaneRouter.openPage("Project", "page", { inPlace: true });
    expect(runGlobalCommand("go/journals")).toBe(true);
    expect(route()).toMatchObject({ kind: "journals" });
  } finally { dispose(); }
});

it("GH #761: sidebar sections and graph switcher share SVG disclosures that follow folding", () => {
  setFavorites([{ name: "Tine", kind: "page" }]);
  setRecentPages([{ name: "Tine", kind: "page" }]);
  const host = document.body.appendChild(document.createElement("div"));
  const dispose = render(() => <Sidebar />, host);
  try {
    const controls = [...host.querySelectorAll<HTMLElement>(".nav-section-toggle")];
    expect(controls.length).toBeGreaterThanOrEqual(2);
    for (const control of controls) {
      const caret = control.querySelector(".nav-toggle-caret")!;
      expect(caret.querySelector("svg")).not.toBeNull();
      const collapsed = caret.classList.contains("collapsed");
      control.click();
      expect(caret.classList.contains("collapsed")).toBe(!collapsed);
      control.click();
    }
    expect(host.querySelector(".graph-switch-caret svg")).not.toBeNull();
  } finally { dispose(); }
});

it("GH #335: All pages opens the full view directly", () => {
  openJournals();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Sidebar />, root);
  try {
    const entry = [...root.querySelectorAll<HTMLElement>(".nav-section-header")].find(el => /all pages/i.test(el.textContent ?? ""))!;
    entry.click();
    expect(route()).toMatchObject({ kind: "all-pages" });
    expect(entry.hasAttribute("aria-expanded")).toBe(false);
    const guide = readFileSync("crates/tine-guide/src/templates/where-things-are.md", "utf8");
    expect(guide).toContain("**All pages** entry opens");
  } finally { dispose(); }
});

afterEach(() => {
  applySidebarSession({ right: false, items: [] });
  resetTabsToJournals();
  setFavorites([]);
  setRecentPages([]);
  setToasts([]);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("graph open recovery (master 9a9122b1544d)", () => {
  it("keeps a known graph's open failure sticky and retries its same target", async () => {
    vi.spyOn(backend(), "listKnownGraphs").mockResolvedValue([
      { name: "Shared notes", path: "\\\\?\\C:\\shared-notes", display_path: "C:\\shared-notes" },
    ]);
    const openKnown = vi.fn(async () => {
      throw new Error("graph root is not readable");
    });
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <GraphSwitcher actions={{
      openKnown,
      openPicked: vi.fn(async () => ({ kind: "aborted" as const })),
      createNew: vi.fn(async () => ({ kind: "aborted" as const })),
    }} />, root);

    try {
      (root.querySelector(".graph-switch-btn") as HTMLButtonElement).click();
      const row = await vi.waitFor(() => {
        const current = root.querySelector<HTMLElement>(".graph-switch-row");
        expect(current).not.toBeNull();
        return current!;
      });
      row.click();
      expect(row.title).toBe("C:\\shared-notes");
      await vi.waitFor(() => expect(toasts()).toHaveLength(1));
      const failure = toasts()[0]!;
      expect(failure.kind).toBe("error");
      expect(failure.message).toContain("graph root is not readable");
      expect(failure.sticky).toBe(true);
      expect(renderEnglish(toUiText(failure.action!.label))).toBe("Retry");

      failure.action!.run();
      await vi.waitFor(() => expect(openKnown).toHaveBeenCalledTimes(2));
      expect(openKnown).toHaveBeenLastCalledWith("\\\\?\\C:\\shared-notes", false);
    } finally {
      dispose();
    }
  });
});

describe("left sidebar section disclosures", () => {
  it("collapses Favorites and Recent independently with semantic controls and counts", async () => {
    setFavorites([
      { name: "Favorite one", kind: "page" },
      { name: "Favorite two", kind: "page" },
    ]);
    setRecentPages([{ name: "Recent one", kind: "page" }]);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <Sidebar />, root);

    try {
      const favorites = await vi.waitFor(() => {
        const control = root.querySelector<HTMLButtonElement>('[data-sidebar-section="favorites"]');
        expect(control).not.toBeNull();
        return control!;
      });
      const recent = root.querySelector<HTMLButtonElement>('[data-sidebar-section="recent"]');
      expect(recent).not.toBeNull();
      expect(favorites.tagName).toBe("BUTTON");
      expect(favorites.getAttribute("aria-expanded")).toBe("true");
      expect(favorites.textContent).toContain("2");
      expect(recent!.getAttribute("aria-expanded")).toBe("true");
      expect(recent!.textContent).toContain("1");
      expect(root.textContent).toContain("Favorite one");
      expect(root.textContent).toContain("Recent one");

      favorites.click();
      expect(favorites.getAttribute("aria-expanded")).toBe("false");
      expect(root.textContent).not.toContain("Favorite one");
      expect(root.textContent).toContain("Recent one");

      recent!.click();
      expect(recent!.getAttribute("aria-expanded")).toBe("false");
      expect(root.textContent).not.toContain("Recent one");

      setFavorites([]);
      setRecentPages([]);
      expect(root.querySelector('[data-sidebar-section="favorites"]')).toBeNull();
      expect(root.querySelector('[data-sidebar-section="recent"]')).toBeNull();
    } finally {
      dispose();
    }
  });
});

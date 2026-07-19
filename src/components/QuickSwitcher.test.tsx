import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { QuickSwitcher } from "./QuickSwitcher";
import { closeSwitcher, openSwitcher, pageInventoryRev, rightSidebar, setRecentPages, setRightSidebar, setRightSidebarOpen, toasts } from "../ui";
import { activeId, closeTab, route, tabRoute, tabs } from "../router";
import { backend } from "../backend";
import { closePane, focusPane, layoutPaneIds, paneRouter, resetPaneLayoutToSingle, setFocusedPaneId, splitPane } from "../panes";
import { loadSingle, resetStore } from "../store";
import type { PageDto } from "../types";

afterEach(() => {
  closeSwitcher();
  setRecentPages([]);
  setRightSidebar([]);
  setRightSidebarOpen(false);
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  resetStore();
  document.body.innerHTML = "";
});

describe("QuickSwitcher search syntax help", () => {
  it("opens an empty-query pathful Recent result at its exact physical owner", async () => {
    const sharedName = "Twin";
    const canonicalPath = "pages/client-a/Twin.md";
    const exactPath = "pages/client-b/Twin.md";
    const canonical: PageDto = {
      name: sharedName,
      kind: "page",
      title: sharedName,
      path: canonicalPath,
      pre_block: null,
      blocks: [{ id: "canonical-twin", raw: "Canonical sibling unchanged", collapsed: false, children: [] }],
    };
    loadSingle(canonical);
    setRecentPages([{ name: sharedName, kind: "page", path: exactPath }]);

    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher();
    try {
      await vi.waitFor(() => {
        const rows = [...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')];
        expect(rows).toHaveLength(1);
        expect(rows[0].textContent).toContain(sharedName);
      });

      root.querySelector<HTMLInputElement>(".switcher-input")!.dispatchEvent(new KeyboardEvent("keydown", {
        key: "Enter", bubbles: true, cancelable: true,
      }));

      expect(route()).toEqual({ kind: "page", name: sharedName, pageKind: "page", path: exactPath });
      expect(canonical.blocks[0].raw).toBe("Canonical sibling unchanged");
    } finally {
      dispose();
    }
  });

  it("opens a selected page in the right sidebar on Shift-only Enter", async () => {
    const search = vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({
      hits: [{
        entity: "page",
        page: { name: "Sidebar target", kind: "page", date_key: null, path: "pages/sidebar-target.md" },
        display_text: "Sidebar target",
        evidence: [{ clause_id: 1, field: "page_name", mode: "fuzzy", spans: [{ start: 0, end: 7 }] }],
        score: 100,
        match_class: "exact",
      }],
      diagnostics: [],
      explanation: { branches: [] },
      cancelled: false,
    });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher();
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.value = "Sidebar target";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => {
      const rows = [...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')];
      expect(rows).toHaveLength(1);
      expect(rows[0].textContent).toContain("Sidebar target");
      expect(rows[0].textContent).not.toContain("Create page:");
    });

    input.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", shiftKey: true, bubbles: true, cancelable: true,
    }));

    expect({ sidebar: rightSidebar(), currentRoute: route(), text: root.textContent }).toMatchObject({
      sidebar: [{ kind: "page", name: "Sidebar target", pageKind: "page", path: "pages/sidebar-target.md" }],
      currentRoute: { kind: "journals" },
    });
    expect(root.querySelector(".switcher")).toBeNull();
    search.mockRestore();
    dispose();
  });

  it.each([
    ["Ctrl", { ctrlKey: true }],
    ["Command", { metaKey: true }],
  ] as const)("does not reinterpret %s-Shift-Enter as sidebar activation", async (_label, modifier) => {
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({
      hits: [{
        entity: "page",
        page: { name: "Ordinary target", kind: "page", date_key: null, path: "pages/ordinary-target.md" },
        display_text: "Ordinary target",
        evidence: [{ clause_id: 1, field: "page_name", mode: "fuzzy", spans: [{ start: 0, end: 8 }] }],
        score: 100,
        match_class: "exact",
      }],
      diagnostics: [], explanation: { branches: [] }, cancelled: false,
    });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher();
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.value = "Ordinary target";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => {
      const rows = [...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')];
      expect(rows).toHaveLength(1);
      expect(rows[0].textContent).toContain("Ordinary target");
      expect(rows[0].textContent).not.toContain("Create page:");
    });

    input.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", shiftKey: true, ...modifier, bubbles: true, cancelable: true,
    }));

    expect(rightSidebar()).toEqual([]);
    expect(route()).toMatchObject({ kind: "page", name: "Ordinary target", pageKind: "page" });
    dispose();
  });

  it("keeps Alt other-pane precedence when Shift is also held", async () => {
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({
      hits: [{
        entity: "page",
        page: { name: "Other-pane target", kind: "page", date_key: null, path: "pages/other-pane-target.md" },
        display_text: "Other-pane target",
        evidence: [{ clause_id: 1, field: "page_name", mode: "fuzzy", spans: [{ start: 0, end: 5 }] }],
        score: 100,
        match_class: "exact",
      }],
      diagnostics: [], explanation: { branches: [] }, cancelled: false,
    });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher();
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.value = "Other-pane target";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => {
      const rows = [...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')];
      expect(rows).toHaveLength(1);
      expect(rows[0].textContent).toContain("Other-pane target");
      expect(rows[0].textContent).not.toContain("Create page:");
    });

    input.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", shiftKey: true, altKey: true, bubbles: true, cancelable: true,
    }));

    const targetPane = layoutPaneIds().find((id) => id !== "main");
    expect(rightSidebar()).toEqual([]);
    expect(route()).toMatchObject({ kind: "journals" });
    expect(targetPane).toBeDefined();
    expect(paneRouter(targetPane!).route()).toMatchObject({
      kind: "page", name: "Other-pane target", pageKind: "page",
    });
    dispose();
  });

  it("opens an unloaded selected block in the sidebar and starts durable target persistence", async () => {
    const blockId = "7eab7af1-1b53-4baa-9082-c1d63540e123";
    const canonicalPath = "pages/unloaded.md";
    const exactPath = "pages/duplicates/unloaded.md";
    const canonical: PageDto = {
      name: "Unloaded", kind: "page", title: "Unloaded", pre_block: null,
      path: canonicalPath, rev: "canonical-rev",
      blocks: [{ id: "canonical-block", raw: "canonical sibling bytes", collapsed: false, children: [] }],
    };
    const exact: PageDto = {
      name: "Unloaded", kind: "page", title: "Unloaded", pre_block: null,
      path: exactPath, rev: "exact-rev",
      blocks: [{ id: blockId, raw: "needle block", collapsed: false, children: [] }],
    };
    const disk = new Map<string, string>([
      [canonicalPath, JSON.stringify(canonical)],
      [exactPath, JSON.stringify(exact)],
    ]);
    const canonicalBytes = disk.get(canonicalPath)!;
    loadSingle(JSON.parse(canonicalBytes) as PageDto);

    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    const getPageByPath = vi.spyOn(backend(), "getPageByPath").mockImplementation(async (path) => {
      const bytes = disk.get(path);
      return bytes ? JSON.parse(bytes) as PageDto : null;
    });
    const savePage = vi.spyOn(backend(), "savePage").mockImplementation(async (dto) => {
      if (dto.path) disk.set(dto.path, JSON.stringify(dto));
      return "saved-exact-rev";
    });
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({
      hits: [{
        entity: "block",
        page: "Unloaded",
        kind: "page",
        path: exactPath,
        block: { id: blockId, raw: "needle block", collapsed: false, children: [], breadcrumb: [] },
        display_text: "needle block",
        evidence: [{ clause_id: 1, field: "visible_content", mode: "contains", spans: [{ start: 0, end: 6 }] }],
        match_class: "body_evidence",
      }],
      diagnostics: [], explanation: { branches: [] }, cancelled: false,
    });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher();
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.value = "needle";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    const blockRow = await vi.waitFor(() => {
      const row = root.querySelector<HTMLElement>(".switcher-row.block-result");
      expect(row).not.toBeNull();
      return row!;
    });
    blockRow.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true, cancelable: true }));

    expect(rightSidebar()).toEqual([{
      kind: "block", uuid: blockId, page: "Unloaded", pageKind: "page",
      path: exactPath,
    }]);
    await vi.waitFor(() => expect(savePage).toHaveBeenCalledTimes(1));
    expect(getPageByPath).toHaveBeenCalledWith(exactPath);
    expect(getPage).not.toHaveBeenCalled();
    const savedExact = JSON.parse(disk.get(exactPath)!) as PageDto;
    expect(savedExact.path).toBe(exactPath);
    expect(savedExact.blocks[0].raw).toBe(`needle block\nid:: ${blockId}`);
    expect(disk.get(canonicalPath)).toBe(canonicalBytes);
    expect(canonical.blocks[0].raw).toBe("canonical sibling bytes");
    dispose();
  });

  it("retains one noncanonical path through current alternate and background page/block activation", async () => {
    const path = "pages/duplicates/Twin.md";
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({
      hits: [
        {
          entity: "page",
          page: { name: "Twin", kind: "page", date_key: null, path },
          display_text: "Twin",
          evidence: [{ clause_id: 1, field: "page_name", mode: "fuzzy", spans: [{ start: 0, end: 4 }] }],
          score: 100,
          match_class: "exact",
        },
        {
          entity: "block",
          page: "Twin",
          kind: "page",
          path,
          block: { id: "exact-block", raw: "owned needle", collapsed: false, children: [], breadcrumb: [] },
          display_text: "owned needle",
          evidence: [{ clause_id: 2, field: "visible_content", mode: "contains", spans: [{ start: 6, end: 12 }] }],
          match_class: "body_evidence",
        },
      ],
      diagnostics: [], explanation: { branches: [] }, cancelled: false,
    });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    const openResults = async () => {
      openSwitcher();
      const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
      input.value = "needle";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      const rows = await vi.waitFor(() => {
        const found = [...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')];
        expect(found).toHaveLength(2);
        return found;
      });
      return { input, pageRow: rows[0], blockRow: rows[1] };
    };

    try {
      const first = await openResults();
      first.pageRow.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 1 }));
      expect(tabs().map(tabRoute)).toContainEqual({ kind: "page", name: "Twin", pageKind: "page", path });

      first.blockRow.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
      first.input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      expect(route()).toMatchObject({ kind: "page", name: "Twin", pageKind: "page", path });

      const alternate = await openResults();
      alternate.blockRow.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
      alternate.input.dispatchEvent(new KeyboardEvent("keydown", {
        key: "Enter", altKey: true, bubbles: true, cancelable: true,
      }));
      const targetPane = layoutPaneIds().find((id) => id !== "main");
      expect(targetPane).toBeDefined();
      expect(paneRouter(targetPane!).route()).toMatchObject({
        kind: "page", name: "Twin", pageKind: "page", path,
      });

      const background = await openResults();
      background.blockRow.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 1 }));
      expect(tabs().map(tabRoute)).toContainEqual({
        kind: "page", name: "Twin", pageKind: "page", block: "exact-block", path,
      });
    } finally {
      dispose();
    }
  });

  it("requests a path-authoritative current-page block scope and hides global providers", async () => {
    resetPaneLayoutToSingle({
      tabs: [{ history: [{ kind: "page", name: "Main", pageKind: "page", path: "pages/Main.md" }], pos: 0, pinned: false }],
      activeIndex: 0,
    });
    const other = splitPane("main", "row")!;
    paneRouter(other).replaceActiveRoute({ kind: "page", name: "Twin", pageKind: "page", path: "pages/second/Twin.md" });
    focusPane(other);
    const search = vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({
      hits: [{
        entity: "block",
        page: "Twin",
        kind: "page",
        block: { id: "block-1", raw: "owned needle", collapsed: true, children: [], breadcrumb: ["Collapsed parent"] },
        display_text: "owned needle",
        evidence: [{ clause_id: 1, field: "visible_content", mode: "contains", spans: [{ start: 6, end: 12 }] }],
        match_class: "body_evidence",
      }],
      diagnostics: [],
      explanation: { branches: [] },
      cancelled: false,
    });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher({ mode: "current-page" });
    await Promise.resolve();
    focusPane("main"); // overlay pointer/focus changes must not retarget the scope
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.value = "needle";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => expect(search).toHaveBeenCalled());

    expect(search).toHaveBeenLastCalledWith(
      "needle", 0, 100, "quick-switch:current-page", false,
      { name: "Twin", pageKind: "page", path: "pages/second/Twin.md" },
    );
    await vi.waitFor(() => expect([...root.querySelectorAll(".switcher-group-header")].map((node) => node.textContent)).toEqual(["Current page1"]));
    expect(root.textContent).not.toContain("Create page:");
    expect(root.querySelector("[data-open-search-tab]")).toBeNull();
    search.mockRestore();
    dispose();
  });

  it("falls back to ordinary global search when current-page search opens from a non-page route", async () => {
    const search = vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({
      hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false,
    });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher({ mode: "current-page" });
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.value = "global";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => expect(search.mock.calls.at(-1)?.[0]).toBe("global"));

    expect(search).toHaveBeenLastCalledWith("global", 100, 100, "quick-switch", false, undefined);
    expect(root.textContent).toContain("Create page: global");
    expect(root.querySelector("[data-open-search-tab]")).not.toBeNull();
    dispose();
  });

  it("suppresses Create when the backend classifies a canonical-equivalent page as exact", async () => {
    vi.spyOn(backend(), "runGraphSearch").mockResolvedValue({
      hits: [{
        entity: "page",
        page: { name: "Cafe\u0301", kind: "page", date_key: null, path: "pages/cafe.md" },
        display_text: "Cafe\u0301",
        evidence: [{ clause_id: 1, field: "page_name", mode: "fuzzy", spans: [{ start: 0, end: 5 }] }],
        score: 1500,
        match_class: "exact",
      }],
      diagnostics: [], explanation: { branches: [] }, cancelled: false,
    });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher();
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.value = "Café";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => expect(root.textContent).toContain("Cafe\u0301"));
    expect(root.textContent).not.toContain("Create page:");
    dispose();
  });

  it("refreshes canonical page inventory after a direct create", async () => {
    const save = vi.spyOn(backend(), "savePage").mockResolvedValue("created-rev");
    const before = pageInventoryRev();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    try {
      openSwitcher();
      const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
      input.value = "Fresh canonical page";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await vi.waitFor(() => {
        expect([...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')]
          .some((row) => row.textContent?.includes("Create page: Fresh canonical page"))).toBe(true);
      });
      const create = [...root.querySelectorAll<HTMLElement>('.switcher-row[role="option"]')]
        .find((row) => row.textContent?.includes("Create page: Fresh canonical page"))!;
      create.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
      expect(pageInventoryRev()).toBeGreaterThan(before);
    } finally {
      save.mockRestore();
      dispose();
    }
  });

  it("opens an empty virtual Search tab from an enabled authoring action", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher();
    const graphSearch = vi.spyOn(backend(), "runGraphSearch");
    await new Promise((resolve) => setTimeout(resolve, 180));
    const openTab = root.querySelector("[data-open-search-tab]") as HTMLButtonElement;
    expect(openTab.disabled).toBe(false);
    expect(openTab.textContent).toContain("Open search tab");
    openTab.click();
    expect(route()).toMatchObject({ kind: "query", sourceKind: "search", source: "", presentation: "search" });
    expect(graphSearch).not.toHaveBeenCalled();
    graphSearch.mockRestore();
    await closeTab(activeId());
    dispose();
  });

  it("keeps a non-main origin despite overlay pointer retargeting and honors embryo/PDF ownership", async () => {
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Main", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    const other = splitPane("main", "row")!;
    focusPane(other);
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher(); await Promise.resolve();
    // The global overlay's capture tracker reports this pointer as main before
    // the button click. The route must still use the snapshot above.
    focusPane("main");
    (root.querySelector("[data-open-search-tab]") as HTMLButtonElement).click();
    expect(paneRouter(other).route()).toMatchObject({ kind: "query", source: "" });

    focusPane("main");
    openSwitcher({ mode: "embryo", paneId: other, prefill: " embryo " }); await Promise.resolve();
    (root.querySelector("[data-open-search-tab]") as HTMLButtonElement).click();
    expect(paneRouter(other).route()).toMatchObject({ kind: "query", source: "embryo" });

    setFocusedPaneId("pdf");
    openSwitcher(); await Promise.resolve();
    (root.querySelector("[data-open-search-tab]") as HTMLButtonElement).click();
    expect(paneRouter("main").route()).toMatchObject({ kind: "query", source: "" });
    dispose();
  });

  it("keeps invalid sources promotable after their debounced diagnostic and exposes a stale-origin retry path", async () => {
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher(); await Promise.resolve();
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    input.value = "/(unclosed/"; input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 180));
    expect((root.querySelector("[data-open-search-tab]") as HTMLButtonElement).disabled).toBe(false);
    closeSwitcher();

    const other = splitPane("main", "row")!;
    focusPane(other); openSwitcher(); await Promise.resolve();
    closePane(other);
    (root.querySelector("[data-open-search-tab]") as HTMLButtonElement).click();
    expect(route().kind).not.toBe("query");
    expect(toasts().at(-1)?.message).toContain("no longer available");
    expect(root.querySelector(".switcher")).not.toBeNull();
    dispose();
  });

  it("is keyboard-accessible and Escape returns focus to search before closing", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher();
    await Promise.resolve();

    const button = root.querySelector(".switcher-syntax-toggle") as HTMLButtonElement;
    const input = root.querySelector(".switcher-input") as HTMLInputElement;
    expect(button.getAttribute("aria-expanded")).toBe("false");
    button.focus();
    button.click();
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(root.querySelectorAll(".switcher-syntax-row")).toHaveLength(5);

    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    await Promise.resolve();
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(input);
    expect(root.querySelector(".switcher")).not.toBeNull();

    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    expect(root.querySelector(".switcher")).toBeNull();
    dispose();
  });

  it("separates block context from a bounded evidence excerpt and exposes combobox semantics", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <QuickSwitcher />, root);
    openSwitcher();
    await Promise.resolve();

    const input = root.querySelector(".switcher-input") as HTMLInputElement;
    input.value = "Tine";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 180));
    await Promise.resolve();

    expect(input.getAttribute("role")).toBe("combobox");
    expect(input.getAttribute("aria-controls")).toBe("switcher-results");
    const block = root.querySelector('.switcher-row[role="option"] .search-result-excerpt');
    expect(block).not.toBeNull();
    expect(block?.parentElement?.querySelector(".search-result-context")).not.toBeNull();
    expect(block?.textContent?.length).toBeLessThanOrEqual(220);

    const openTab = root.querySelector("[data-open-search-tab]") as HTMLButtonElement;
    expect(openTab.disabled).toBe(false);
    openTab.click();
    expect(route()).toMatchObject({ kind: "query", source: "Tine", presentation: "search" });
    await closeTab(activeId());

    dispose();
  });
});

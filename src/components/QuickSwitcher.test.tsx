import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { QuickSwitcher } from "./QuickSwitcher";
import { closeSwitcher, openSwitcher, toasts } from "../ui";
import { activeId, closeTab, route } from "../router";
import { backend } from "../backend";
import { closePane, focusPane, paneRouter, resetPaneLayoutToSingle, setFocusedPaneId, splitPane } from "../panes";

afterEach(() => {
  closeSwitcher();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  document.body.innerHTML = "";
});

describe("QuickSwitcher search syntax help", () => {
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

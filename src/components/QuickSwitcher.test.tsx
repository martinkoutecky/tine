import { afterEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { QuickSwitcher } from "./QuickSwitcher";
import { closeSwitcher, openSwitcher } from "../ui";
import { activeId, closeTab, route } from "../router";

afterEach(() => {
  closeSwitcher();
  document.body.innerHTML = "";
});

describe("QuickSwitcher search syntax help", () => {
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

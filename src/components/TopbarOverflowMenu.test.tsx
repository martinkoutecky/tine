import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { clearTransientLayersForTest, dismissTopTransient } from "../transientLayers";
import { TopbarOverflowMenu, type TopbarOverflowMenuProps } from "./TopbarOverflowMenu";

let dispose = () => {};

afterEach(() => {
  dispose();
  dispose = () => {};
  clearTransientLayersForTest();
  document.body.innerHTML = "";
});

function mount(actions: Partial<TopbarOverflowMenuProps> = {}) {
  const defaults: TopbarOverflowMenuProps = {
    onCalendar: vi.fn(),
    onJournals: vi.fn(),
    onToggleTheme: vi.fn(),
    onToggleRightSidebar: vi.fn(),
    onBack: vi.fn(),
    onForward: vi.fn(),
    canGoBack: () => true,
    canGoForward: () => true,
    ...actions,
  };
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <TopbarOverflowMenu {...defaults} />, host);
  return { host, actions: defaults };
}

describe("TopbarOverflowMenu", () => {
  it("runs the same shared action handlers for every collapsed toolbar control", () => {
    const { host, actions } = mount();
    const trigger = host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!;

    for (const [id, handler] of [
      ["calendar", actions.onCalendar],
      ["journals", actions.onJournals],
      ["theme", actions.onToggleTheme],
      ["right-sidebar", actions.onToggleRightSidebar],
      ["back", actions.onBack],
      ["forward", actions.onForward],
    ] as const) {
      trigger.click();
      host.querySelector<HTMLButtonElement>(`[data-topbar-overflow-action="${id}"]`)!.click();
      expect(handler).toHaveBeenCalledTimes(1);
      expect(host.querySelector(".topbar-overflow-menu")).toBeNull();
    }
  });

  it("is a transient-layer popover that dismisses and restores trigger focus", () => {
    const { host } = mount();
    const trigger = host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!;
    trigger.click();
    expect(host.querySelector('[role="menu"]')).not.toBeNull();

    expect(dismissTopTransient("escape")).toBe(true);
    expect(host.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("dismisses on a mousedown outside the menu (GH #205 follow-up)", () => {
    const { host } = mount();
    const trigger = host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!;
    trigger.click();
    expect(host.querySelector('[role="menu"]')).not.toBeNull();

    // A click somewhere else on the page (here: document.body) must close it.
    const outside = document.createElement("button");
    document.body.appendChild(outside);
    outside.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    expect(host.querySelector('[role="menu"]')).toBeNull();
  });

  it("a mousedown INSIDE the menu does not dismiss it", () => {
    const { host } = mount();
    const trigger = host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!;
    trigger.click();
    const item = host.querySelector<HTMLButtonElement>('[data-topbar-overflow-action="theme"]')!;
    item.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    expect(host.querySelector('[role="menu"]')).not.toBeNull();
  });
});

import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { clearTransientLayersForTest, dismissTopTransient } from "../transientLayers";
import { TopbarOverflowMenu, type TopbarOverflowMenuProps } from "./TopbarOverflowMenu";
import { toolbarActions } from "../toolbarChoice";

let dispose = () => {};

afterEach(() => {
  dispose();
  dispose = () => {};
  clearTransientLayersForTest();
  document.body.innerHTML = "";
});

function mount(actions: Partial<TopbarOverflowMenuProps> = {}) {
  const defaults: TopbarOverflowMenuProps = {
    actions: toolbarActions.map(action => action.id), visible: true,
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

  it("keeps the chosen optional order and ends with Customize toolbar (GH #926)", () => {
    const onSettings = vi.fn();
    const onCustomize = vi.fn();
    const { host } = mount({ onSettings, onShare: vi.fn(), actions: ["theme", "settings"], onCustomize });
    host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!.click();
    const rows = [...host.querySelectorAll<HTMLButtonElement>("[data-topbar-overflow-action]")].filter(row => !["back", "forward"].includes(row.dataset.topbarOverflowAction!));
    expect(rows.map(row => row.dataset.topbarOverflowAction)).toEqual(["theme", "settings"]);
    expect(host.querySelector('[role="menu"] button:last-child')!.textContent).toBe("Customize toolbar…");
    rows[1].click();
    expect(onSettings).toHaveBeenCalledOnce();
    expect(host.querySelector(".topbar-overflow-menu")).toBeNull();
  });

  it("omits the Settings row where Settings does not apply (a published export)", () => {
    const { host } = mount();
    host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!.click();
    expect(host.querySelector('[data-topbar-overflow-action="settings"]')).toBeNull();
  });

  it("offers Share as the twin of the topbar button: shown only when wired, disabled with no page", () => {
    const bare = mount();
    bare.host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!.click();
    expect(bare.host.querySelector('[data-topbar-overflow-action="share"]')).toBeNull();
    dispose(); dispose = () => {}; document.body.innerHTML = "";

    const none = mount({ onShare: vi.fn(), canShare: () => false });
    none.host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!.click();
    expect(none.host.querySelector<HTMLButtonElement>('[data-topbar-overflow-action="share"]')!.disabled).toBe(true);
    dispose(); dispose = () => {}; document.body.innerHTML = "";

    const { host, actions } = mount({ onShare: vi.fn(), canShare: () => true });
    host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!.click();
    host.querySelector<HTMLButtonElement>('[data-topbar-overflow-action="share"]')!.click();
    expect(actions.onShare).toHaveBeenCalledTimes(1);
    expect(host.querySelector(".topbar-overflow-menu")).toBeNull();
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

  // GH #472 (master 8198daf34): "the user pressed outside" has one producer.
  // A touch or pen press delivers pointerdown; this menu listened only for
  // mousedown, so it stayed open when tapped away from.
  it("closes on an outside pointerdown, and a press on its trigger still toggles", () => {
    const { host } = mount();
    const outside = document.createElement("button");
    document.body.appendChild(outside);
    const trigger = host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!;
    trigger.click();
    expect(host.querySelector(".topbar-overflow-menu")).not.toBeNull();
    outside.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    expect(host.querySelector(".topbar-overflow-menu")).toBeNull();

    trigger.click();
    trigger.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    trigger.dispatchEvent(new Event("mousedown", { bubbles: true }));
    expect(host.querySelector(".topbar-overflow-menu")).not.toBeNull();
    trigger.click();
    expect(host.querySelector(".topbar-overflow-menu")).toBeNull();
  });
});

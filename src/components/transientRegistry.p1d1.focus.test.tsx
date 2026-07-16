import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { installKeybindings } from "../keybindings";
import { closeSettings, openSettings } from "../ui";
import { clearTransientLayersForTest, dismissTopTransient, registerTransientLayer } from "../transientLayers";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const marker = "P1D1-B-FOCUS-WITNESS: registry overwrote the Advanced trigger";

function mountDrawer() {
  const active = document.createElement("div");
  active.dataset.activeDrawer = "left";
  const drawer = document.createElement("aside");
  drawer.className = "left-sidebar";
  active.append(drawer);
  document.body.append(active);
  return drawer;
}

async function dismissAndFlush() {
  expect(dismissTopTransient("escape")).toBe(true);
  await tick();
}

afterEach(() => {
  closeSettings();
  clearTransientLayersForTest();
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("GH #161 P1D1-B exact trigger focus restoration", () => {
  it("focuses the valid exact trigger of the dismissed entry", async () => {
    const trigger = document.createElement("button");
    document.body.append(trigger);
    const unregister = registerTransientLayer({ id: "dismissed", trigger: () => trigger, dismiss: () => true });
    try {
      await dismissAndFlush();
      expect(document.activeElement).toBe(trigger);
    } finally {
      unregister();
    }
  });

  it("skips a disconnected dismissed trigger and reaches the drawer", async () => {
    const trigger = document.createElement("button");
    const drawer = mountDrawer();
    const fallback = document.createElement("button");
    drawer.append(fallback);
    const unregister = registerTransientLayer({ id: "dismissed", trigger: () => trigger, dismiss: () => true });
    try {
      await dismissAndFlush();
      expect(document.activeElement).toBe(fallback);
    } finally {
      unregister();
    }
  });

  const invalidCandidates: Array<{
    label: string;
    tag: "button" | "div";
    prepare: (trigger: HTMLElement, wrapper: HTMLElement) => void;
  }> = [
    { label: "self inert", tag: "button", prepare: (trigger) => trigger.setAttribute("inert", "") },
    { label: "inert ancestor", tag: "button", prepare: (_trigger, wrapper) => wrapper.setAttribute("inert", "") },
    { label: "self hidden", tag: "button", prepare: (trigger) => { trigger.hidden = true; } },
    { label: "hidden ancestor", tag: "button", prepare: (_trigger, wrapper) => { wrapper.hidden = true; } },
    { label: "self aria-hidden", tag: "button", prepare: (trigger) => trigger.setAttribute("aria-hidden", "true") },
    { label: "aria-hidden ancestor", tag: "button", prepare: (_trigger, wrapper) => wrapper.setAttribute("aria-hidden", "true") },
    { label: "disabled native control", tag: "button", prepare: (trigger) => (trigger as HTMLButtonElement).disabled = true },
    { label: "plain nonfocusable element", tag: "div", prepare: () => {} },
  ];

  it.each(invalidCandidates)("never attempts focus for an invalid dismissed trigger: $label", async ({ tag, prepare }) => {
    const wrapper = document.createElement("div");
    const trigger = document.createElement(tag);
    wrapper.append(trigger);
    document.body.append(wrapper);
    prepare(trigger, wrapper);
    const attempted = vi.spyOn(trigger, "focus");
    const drawer = mountDrawer();
    const fallback = document.createElement("button");
    drawer.append(fallback);
    const unregister = registerTransientLayer({ id: "dismissed", trigger: () => trigger, dismiss: () => true });
    try {
      await dismissAndFlush();
      expect(attempted).not.toHaveBeenCalled();
      expect(document.activeElement).toBe(fallback);
    } finally {
      unregister();
    }
  });

  it("uses the surviving top trigger before its root descendants", async () => {
    const survivingTrigger = document.createElement("button");
    const root = document.createElement("div");
    const descendant = document.createElement("button");
    root.append(descendant);
    document.body.append(survivingTrigger, root);
    const unregisterSurviving = registerTransientLayer({
      id: "surviving",
      trigger: () => survivingTrigger,
      root: () => root,
      dismiss: () => true,
    });
    const disconnected = document.createElement("button");
    let unregisterDismissed = () => {};
    unregisterDismissed = registerTransientLayer({
      id: "dismissed",
      trigger: () => disconnected,
      dismiss: () => { unregisterDismissed(); return true; },
    });
    const descendantFocus = vi.spyOn(descendant, "focus");
    try {
      await dismissAndFlush();
      expect(document.activeElement).toBe(survivingTrigger);
      expect(descendantFocus).not.toHaveBeenCalled();
    } finally {
      unregisterDismissed();
      unregisterSurviving();
    }
  });

  it.each(["native-first", "tabindex-first"] as const)("enumerates native and tabindex descendants in DOM order: %s", async (order) => {
    const root = document.createElement("div");
    const native = document.createElement("button");
    const programmatic = document.createElement("div");
    programmatic.tabIndex = -1;
    if (order === "native-first") root.append(native, programmatic);
    else root.append(programmatic, native);
    document.body.append(root);
    const disconnected = document.createElement("button");
    const unregister = registerTransientLayer({ id: "top", trigger: () => disconnected, root: () => root, dismiss: () => true });
    try {
      await dismissAndFlush();
      expect(document.activeElement).toBe(order === "native-first" ? native : programmatic);
    } finally {
      unregister();
    }
  });

  it("uses an explicitly programmatically focusable surviving root", async () => {
    const root = document.createElement("div");
    root.tabIndex = -1;
    document.body.append(root);
    const disconnected = document.createElement("button");
    const unregister = registerTransientLayer({ id: "top", trigger: () => disconnected, root: () => root, dismiss: () => true });
    try {
      await dismissAndFlush();
      expect(document.activeElement).toBe(root);
    } finally {
      unregister();
    }
  });

  it.each(["pre", "div"] as const)("does not let a plain %s root stop drawer fallback", async (tag) => {
    const root = document.createElement(tag);
    document.body.append(root);
    const rootFocus = vi.spyOn(root, "focus");
    const drawer = mountDrawer();
    const fallback = document.createElement("button");
    drawer.append(fallback);
    const disconnected = document.createElement("button");
    const unregister = registerTransientLayer({ id: "top", trigger: () => disconnected, root: () => root, dismiss: () => true });
    try {
      await dismissAndFlush();
      expect(rootFocus).not.toHaveBeenCalled();
      expect(document.activeElement).toBe(fallback);
    } finally {
      unregister();
    }
  });

  it("uses active drawer descendants, then its explicitly focusable root", async () => {
    const disconnected = document.createElement("button");
    const drawer = mountDrawer();
    const descendant = document.createElement("button");
    drawer.append(descendant);
    const unregister = registerTransientLayer({ id: "top", trigger: () => disconnected, dismiss: () => true });
    try {
      await dismissAndFlush();
      expect(document.activeElement).toBe(descendant);
      descendant.remove();
      drawer.tabIndex = -1;
      await dismissAndFlush();
      expect(document.activeElement).toBe(drawer);
    } finally {
      unregister();
    }
  });

  it("continues after an eligible focus attempt that does not become active", async () => {
    const root = document.createElement("div");
    const ignored = document.createElement("button");
    const fallback = document.createElement("button");
    root.append(ignored, fallback);
    document.body.append(root);
    const ignoredFocus = vi.spyOn(ignored, "focus").mockImplementation(() => {});
    const disconnected = document.createElement("button");
    const unregister = registerTransientLayer({ id: "top", trigger: () => disconnected, root: () => root, dismiss: () => true });
    try {
      await dismissAndFlush();
      expect(ignoredFocus).toHaveBeenCalledOnce();
      expect(document.activeElement).toBe(fallback);
    } finally {
      unregister();
    }
  });

  it("restores the same entry trigger after an internal visibility-only rung", async () => {
    const trigger = document.createElement("button");
    document.body.append(trigger);
    let visible = true;
    const triggerFocus = vi.spyOn(trigger, "focus");
    const unregister = registerTransientLayer({
      id: "same-entry",
      trigger: () => trigger,
      dismiss: () => { visible = false; return true; },
    });
    try {
      await dismissAndFlush();
      expect(visible).toBe(false);
      expect(triggerFocus).toHaveBeenCalledOnce();
      expect(document.activeElement).toBe(trigger);
    } finally {
      unregister();
    }
  });

  it("closes only real Settings Advanced and restores its exact toggle after queued work", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <Settings />, host);
    const uninstall = installKeybindings();
    try {
      openSettings("editor");
      await tick();
      const advanced = host.querySelector<HTMLButtonElement>(".settings-advanced-toggle")!;
      advanced.click();
      await tick();

      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
      await tick();
      if (document.activeElement !== advanced) throw new Error(marker);
      expect(host.querySelector(".settings-modal")).not.toBeNull();
      expect(advanced.getAttribute("aria-expanded")).toBe("false");
    } finally {
      uninstall();
      dispose();
    }
  });
});

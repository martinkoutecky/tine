import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { HelpPopup } from "./HelpShortcuts";
import { Settings } from "./Settings";
import { installKeybindings } from "../keybindings";
import { closeHelpPopup, closeSettings, openSettings, toggleHelpPopup } from "../ui";
import {
  clearTransientLayersForTest,
  registerTransientLayer,
  setTransientLayerTokenForTest,
  topTransientLayer,
} from "../transientLayers";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function escape() {
  return new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
}

function mountSettings() {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <Settings />, host);
  return { host, dispose };
}

afterEach(() => {
  closeHelpPopup();
  closeSettings();
  clearTransientLayersForTest();
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("GH #161 P1D1-A semantic branch ordering", () => {
  it("lets an independently activated real Help owner win, then routes parent-only Settings focus to its live Advanced child", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <><Settings /><HelpPopup /></>, host);
    const uninstall = installKeybindings();
    openSettings("editor");
    await tick();
    const advanced = host.querySelector<HTMLButtonElement>(".settings-advanced-toggle")!;
    advanced.click();
    await tick();
    toggleHelpPopup();
    await tick();

    window.dispatchEvent(escape());
    await tick();
    expect(host.querySelector(".help-menu")).toBeNull();
    expect(advanced.getAttribute("aria-expanded")).toBe("true");

    toggleHelpPopup();
    await tick();
    const parentOnly = host.querySelector<HTMLInputElement>(".settings-search-input")!;
    parentOnly.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    window.dispatchEvent(escape());
    await tick();

    expect(advanced.getAttribute("aria-expanded")).toBe("false");
    expect(host.querySelector(".settings-modal")).not.toBeNull();
    expect(host.querySelector(".help-menu")).not.toBeNull();
    uninstall();
    dispose();
  });

  it("routes both parent-only focus and pointer activation to the live Advanced child", async () => {
    const { host, dispose } = mountSettings();
    const uninstall = installKeybindings();
    openSettings("editor");
    await tick();
    const advanced = host.querySelector<HTMLButtonElement>(".settings-advanced-toggle")!;
    const parentOnly = host.querySelector<HTMLInputElement>(".settings-search-input")!;
    advanced.click();
    await tick();

    parentOnly.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    window.dispatchEvent(escape());
    await tick();
    expect(advanced.getAttribute("aria-expanded")).toBe("false");

    advanced.click();
    await tick();
    parentOnly.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    window.dispatchEvent(escape());
    await tick();
    expect(advanced.getAttribute("aria-expanded")).toBe("false");
    uninstall();
    dispose();
  });

  it("selects either concrete Settings child, retains the newest after parent activity, then selects the remaining sibling before Settings", async () => {
    const { host, dispose } = mountSettings();
    const uninstall = installKeybindings();
    openSettings("editor");
    await tick();
    const advanced = host.querySelector<HTMLButtonElement>(".settings-advanced-toggle")!;
    const parentOnly = host.querySelector<HTMLInputElement>(".settings-search-input")!;
    advanced.click();
    await tick();
    const section = host.querySelector<HTMLElement>(".settings-advanced")!;
    const synthetic = document.createElement("button");
    synthetic.textContent = "Synthetic Settings child";
    section.append(synthetic);
    const disposeSynthetic = registerTransientLayer({
      id: "settings-synthetic-child",
      parentId: "settings",
      root: () => synthetic,
      dismiss: () => false,
    });

    section.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    const advancedId = topTransientLayer()?.id;
    expect(advancedId).toMatch(/^settings-advanced-/);
    synthetic.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    expect(topTransientLayer()?.id).toBe("settings-synthetic-child");
    parentOnly.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    expect(topTransientLayer()?.id).toBe("settings-synthetic-child");
    window.dispatchEvent(escape());
    await tick();
    expect(topTransientLayer()?.id).toBe(advancedId);
    window.dispatchEvent(escape());
    await tick();
    expect(advanced.getAttribute("aria-expanded")).toBe("false");
    expect(topTransientLayer()?.id).toBe("settings");
    disposeSynthetic();
    expect(topTransientLayer()?.id).toBe("settings");
    uninstall();
    dispose();
  });

  it("normalizes every parent-cycle member to a root, keeps descendants attached, and breaks token ties by greatest stable id", () => {
    const a = document.createElement("button");
    const b = document.createElement("button");
    const descendant = document.createElement("button");
    const missing = document.createElement("button");
    document.body.append(a, b, descendant, missing);
    const disposeA = registerTransientLayer({ id: "A", parentId: "B", root: () => a, dismiss: () => true });
    const disposeB = registerTransientLayer({ id: "B", parentId: "A", root: () => b, dismiss: () => true });
    const disposeDescendant = registerTransientLayer({ id: "descendant", parentId: "A", root: () => descendant, dismiss: () => true });
    const disposeMissing = registerTransientLayer({ id: "missing", parentId: "absent", root: () => missing, dismiss: () => true });
    // A and B must both become roots: A wins the branch race, then its
    // still-attached low-token descendant is the leaf. Cutting only A's edge
    // would instead select B; detaching descendant would instead select A.
    setTransientLayerTokenForTest("A", 10);
    setTransientLayerTokenForTest("B", 9);
    setTransientLayerTokenForTest("descendant", 5);
    setTransientLayerTokenForTest("missing", 8);
    expect(topTransientLayer()?.id).toBe("descendant");

    // With equal root-branch tokens, the lexicographically greatest root id
    // wins. Together with the phase above, this rejects either one-edge cycle
    // normalization.
    setTransientLayerTokenForTest("A", 9);
    expect(topTransientLayer()?.id).toBe("B");

    setTransientLayerTokenForTest("missing", 10);
    expect(topTransientLayer()?.id).toBe("missing");
    disposeMissing();
    expect(topTransientLayer()?.id).toBe("B");
    disposeDescendant();
    disposeB();
    disposeA();

    const root = document.createElement("button");
    const childA = document.createElement("button");
    const childB = document.createElement("button");
    document.body.append(root, childA, childB);
    const disposeRoot = registerTransientLayer({ id: "root", root: () => root, dismiss: () => true });
    const disposeChildA = registerTransientLayer({ id: "child-a", parentId: "root", root: () => childA, dismiss: () => true });
    const disposeChildB = registerTransientLayer({ id: "child-b", parentId: "root", root: () => childB, dismiss: () => true });
    setTransientLayerTokenForTest("root", 1);
    setTransientLayerTokenForTest("child-a", 11);
    setTransientLayerTokenForTest("child-b", 11);
    expect(topTransientLayer()?.id).toBe("child-b");
    disposeChildB();
    disposeChildA();
    disposeRoot();
  });

  it("bypasses composing/229 Escape and consumes one ordinary Escape without falling through", () => {
    const target = document.createElement("button");
    document.body.append(target);
    const top = vi.fn(() => false);
    const lower = vi.fn(() => true);
    const disposeTop = registerTransientLayer({ id: "top", root: () => target, dismiss: top });
    const disposeLower = registerTransientLayer({ id: "lower", dismiss: lower });
    // Make the false-pruned top literal top despite lower's later registration.
    setTransientLayerTokenForTest("top", 9);
    const uninstall = installKeybindings();

    const composing = escape();
    Object.defineProperty(composing, "isComposing", { value: true });
    target.dispatchEvent(composing);
    const legacy = escape();
    Object.defineProperty(legacy, "keyCode", { value: 229 });
    target.dispatchEvent(legacy);
    expect(top).not.toHaveBeenCalled();
    expect(lower).not.toHaveBeenCalled();

    const ordinary = escape();
    target.dispatchEvent(ordinary);
    expect(top).toHaveBeenCalledTimes(1);
    expect(lower).not.toHaveBeenCalled();
    expect(ordinary.defaultPrevented).toBe(true);
    uninstall();
    disposeLower();
    disposeTop();
  });
});

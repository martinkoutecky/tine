import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { DatePicker } from "./DatePicker";
import { ContextMenu } from "./ContextMenu";
import { Welcome } from "./Welcome";
import { installKeybindings, setKeybindingsSuspended } from "../keybindings";
import { closeContextMenu, closeDatePicker, contextMenu, datePicker, openContextMenu, openDatePicker } from "../ui";
import { clearTransientLayersForTest, registerTransientLayer } from "../transientLayers";
import { loadSingle, resetStore } from "../store";
import { initParser } from "../render/parse";

function escape(init: { composing?: boolean; keyCode?: number } = {}) {
  const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
  if (init.composing) Object.defineProperty(event, "isComposing", { value: true });
  if (init.keyCode != null) Object.defineProperty(event, "keyCode", { value: init.keyCode });
  return event;
}

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  setKeybindingsSuspended(false);
  closeDatePicker();
  closeContextMenu();
  clearTransientLayersForTest();
  resetStore();
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("GH #161 P1 capture transient dispatch", () => {
  it("mounts DatePicker as the capture WebView owner: Escape closes it before a lower cancellation rung", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <DatePicker />, host);
    const lower = vi.fn(() => true);
    registerTransientLayer({ id: "capture-cancel-sentinel", dismiss: lower });
    openDatePicker("capture-block", "scheduled", 12, 24);

    const uninstall = installKeybindings();
    const event = escape();
    window.dispatchEvent(event);

    expect(datePicker()).toBeNull();
    expect(lower).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(true);
    uninstall();
    dispose();
  });

  it("routes Escape through the mounted DatePicker before shortcut-recording suspension, but leaves composing Escape alone", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <DatePicker />, host);
    const uninstall = installKeybindings();
    openDatePicker("capture-block", "scheduled", 12, 24);
    setKeybindingsSuspended(true);

    const composing = escape({ composing: true });
    window.dispatchEvent(composing);
    expect(datePicker()).not.toBeNull();
    expect(composing.defaultPrevented).toBe(false);

    const event = escape();
    window.dispatchEvent(event);
    expect(datePicker()).toBeNull();
    expect(event.defaultPrevented).toBe(true);

    uninstall();
    dispose();
  });

  it("peels the real ContextMenu template-name editor without closing its menu", () => {
    loadSingle({
      name: "P1",
      kind: "page",
      title: "P1",
      pre_block: null,
      blocks: [{ id: "template-block", raw: "Template", collapsed: false, children: [] }],
    });
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <ContextMenu />, host);
    openContextMenu(12, 24, "template-block");
    const start = [...host.querySelectorAll<HTMLElement>(".ctx-item")].find((el) => el.textContent?.includes("Make a template"));
    expect(start).toBeTruthy();
    start!.click();
    expect(host.querySelector(".ctx-template-name")).not.toBeNull();

    const uninstall = installKeybindings();
    window.dispatchEvent(escape());

    expect(host.querySelector(".ctx-template-name")).toBeNull();
    expect(contextMenu()).not.toBeNull();
    uninstall();
    dispose();
  });

  it("uses production focus/pointer activation to bring an older mounted root forward", () => {
    const older = document.createElement("button");
    const newer = document.createElement("button");
    document.body.append(older, newer);
    const dismissed: string[] = [];
    registerTransientLayer({ id: "older", root: () => older, dismiss: () => { dismissed.push("older"); return true; } });
    registerTransientLayer({ id: "newer", root: () => newer, dismiss: () => { dismissed.push("newer"); return true; } });
    older.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));

    const uninstall = installKeybindings();
    window.dispatchEvent(escape());

    expect(dismissed).toEqual(["older"]);
    uninstall();
  });

  it("does not register mandatory first-load Welcome as an endlessly consuming transient", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <Welcome />, host);
    const lower = vi.fn(() => true);
    registerTransientLayer({ id: "root-fallback-sentinel", dismiss: lower });

    const uninstall = installKeybindings();
    window.dispatchEvent(escape());

    expect(lower).toHaveBeenCalledTimes(1);
    uninstall();
    dispose();
  });
});

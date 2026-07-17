import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { editingId, startEditing } from "./editorController";
import { installKeybindings, setKeybindingsSuspended } from "./keybindings";
import { initParser } from "./render/parse";
import { clearSelection, loadSingle, resetStore } from "./store";
import type { BlockDto, PageDto } from "./types";
import { focusMode, setFocusMode } from "./ui";
import { clearTransientLayersForTest, registerTransientLayer } from "./transientLayers";
import { pageByName } from "./store";
import { Block } from "./components/Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  setKeybindingsSuspended(false);
  setFocusMode(false);
  clearSelection();
  clearTransientLayersForTest();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(node, root) };
}

function page(name: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "page", title: name, pre_block: null, blocks };
}

function escape(init: { composing?: boolean; keyCode?: number } = {}) {
  const event = new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true, cancelable: true });
  if (init.composing) Object.defineProperty(event, "isComposing", { value: true });
  if (init.keyCode != null) Object.defineProperty(event, "keyCode", { value: init.keyCode });
  return event;
}

describe("GH #161 P1A1 global capture suspension boundary", () => {
  it("keeps a suspended non-editable focus rung unchanged and consumes Escape", () => {
    const target = document.createElement("button");
    document.body.append(target);
    setFocusMode(true);
    setKeybindingsSuspended(true);
    const uninstall = installKeybindings();

    const event = escape();
    target.dispatchEvent(event);

    expect(focusMode()).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    uninstall();
  });

  it("keeps a suspended real Block editor and its target-phase sentinel unchanged", () => {
    loadSingle(page("P1A1", [{ id: "block", raw: "Text", collapsed: false, children: [] }]));
    startEditing("block", 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("P1A1")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    const editor = root.querySelector("textarea.block-editor") as HTMLTextAreaElement | null;
    expect(editor).not.toBeNull();
    const targetPhase = vi.fn();
    editor!.addEventListener("keydown", targetPhase);
    setKeybindingsSuspended(true);
    const uninstall = installKeybindings();

    const event = escape();
    editor!.dispatchEvent(event);

    expect(editingId()).toBe("block");
    expect(targetPhase).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(true);
    uninstall();
    dispose();
  });

  it("still lets a suspended Escape dismiss exactly one registered transient", () => {
    const target = document.createElement("button");
    document.body.append(target);
    const dismiss = vi.fn(() => true);
    registerTransientLayer({ id: "p1a1-transient", dismiss });
    setKeybindingsSuspended(true);
    const uninstall = installKeybindings();

    const event = escape();
    target.dispatchEvent(event);

    expect(dismiss).toHaveBeenCalledTimes(1);
    expect(event.defaultPrevented).toBe(true);
    uninstall();
  });

  it("declines composing and keyCode-229 Escape before the global transient prefix", () => {
    const target = document.createElement("button");
    document.body.append(target);
    const dismiss = vi.fn(() => true);
    registerTransientLayer({ id: "p1a1-ime", dismiss });
    const uninstall = installKeybindings();

    const composing = escape({ composing: true });
    target.dispatchEvent(composing);
    const legacy = escape({ keyCode: 229 });
    target.dispatchEvent(legacy);

    expect(dismiss).not.toHaveBeenCalled();
    expect(composing.defaultPrevented).toBe(false);
    expect(legacy.defaultPrevented).toBe(false);
    uninstall();
  });

  it("retains the unsuspended Block edit-to-selection behavior", () => {
    loadSingle(page("Unsuspended", [{ id: "block", raw: "Text", collapsed: false, children: [] }]));
    startEditing("block", 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Unsuspended")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    const editor = root.querySelector("textarea.block-editor") as HTMLTextAreaElement | null;
    expect(editor).not.toBeNull();
    const uninstall = installKeybindings();

    editor!.dispatchEvent(escape());

    expect(editingId()).toBeNull();
    uninstall();
    dispose();
  });
});

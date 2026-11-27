// A modal dialog owns the keys pressed in it. With a block selected on the page
// behind, Delete, Backspace or Ctrl+X in the recovery panel or another dialog
// must neither remove the block nor submit a save; the same key on the page
// still acts on the selection.
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { initParser } from "./render/parse";
import { loadSingle } from "./document/workingSet";
import { node, resetStore, selectBlock, selectedIds } from "./document";
import { clearClipboardPayload } from "./clipboard";
import { setGraphMeta } from "./graphSession";
import { clearTransientLayersForTest, registerTransientLayer } from "./transientLayers";
import { installKeybindings } from "./keybindings";
import { bindTestHost } from "./document/host/wiring.test.support";
import { UnsavedRecovery } from "./components/UnsavedRecovery";
import { closeUnsavedRecovery, openUnsavedRecovery } from "./unsavedRecovery";
import { GraphNamePrompt } from "./components/GraphNamePrompt";
import { askGraphName } from "./graphNamePrompt";

const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
const KEYS: [string, KeyboardEventInit][] = [
  ["Delete", { key: "Delete", code: "Delete" }],
  ["Backspace", { key: "Backspace", code: "Backspace" }],
  ["Ctrl+X", { key: "x", code: "KeyX", ctrlKey: true }],
];

let disposers: (() => void)[] = [];
beforeAll(() => initParser());
beforeEach(async () => {
  await bindTestHost();
  vi.spyOn(backend(), "pageKeptOriginals").mockResolvedValue([]);
  vi.spyOn(backend(), "writeRich").mockResolvedValue();
  vi.spyOn(backend(), "writeText").mockResolvedValue();
  loadSingle({
    name: "P", kind: "page", title: "P", format: "md", pre_block: null,
    blocks: [{ id: "A", raw: "A", children: [], collapsed: false }, { id: "B", raw: "B", children: [], collapsed: false }],
  });
  setGraphMeta({ root: "/fixture" } as any);
  disposers.push(installKeybindings());
});
afterEach(() => {
  for (const dispose of disposers.splice(0).reverse()) dispose();
  closeUnsavedRecovery(); clearClipboardPayload(); resetStore();
  setGraphMeta(null); clearTransientLayersForTest(); vi.restoreAllMocks();
  document.body.innerHTML = "";
});

function press(target: Element, init: KeyboardEventInit): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { ...init, bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

/** Leaves B untouched: still in the outline, no save submitted, nothing cut. */
async function expectUntouched(): Promise<void> {
  await flush();
  expect(node("B")).toBeTruthy();
  expect(backend().pageSubmit).not.toHaveBeenCalled();
  expect(backend().writeRich).not.toHaveBeenCalled();
}

async function openRecovery(): Promise<Element> {
  const root = document.createElement("div");
  document.body.append(root);
  openUnsavedRecovery();
  disposers.push(render(() => <UnsavedRecovery />, root));
  await flush();
  return root.querySelector("[role=dialog]")!;
}

async function openGraphName(): Promise<Element> {
  const root = document.createElement("div");
  document.body.append(root);
  disposers.push(render(() => <GraphNamePrompt />, root));
  void askGraphName("notes", vi.fn(async () => null));
  disposers.push(() => [...root.querySelectorAll("button")].find((b) => b.textContent === "Cancel")?.click());
  await flush();
  return root.querySelector("[role=dialog]")!;
}

it.each(KEYS)("%s in the recovery panel leaves the selected block behind it alone", async (_name, init) => {
  vi.spyOn(backend(), "pageSubmit");
  selectBlock("B");
  const dialog = await openRecovery();
  expect(document.activeElement).toBe(dialog);
  expect(press(dialog, init).defaultPrevented).toBe(false);
  await expectUntouched();
  expect(selectedIds()).toEqual(["B"]);
});

it.each(KEYS)("%s on a button of the new-graph dialog leaves the selected block alone", async (_name, init) => {
  vi.spyOn(backend(), "pageSubmit");
  selectBlock("B");
  const dialog = await openGraphName();
  const cancel = [...dialog.querySelectorAll("button")].find((b) => b.textContent === "Cancel")!;
  cancel.focus();
  expect(press(cancel, init).defaultPrevented).toBe(false);
  await expectUntouched();
});

it("the open dialog owns the key even when focus is outside it", async () => {
  vi.spyOn(backend(), "pageSubmit");
  selectBlock("B");
  await openRecovery();
  press(document.body, { key: "Delete", code: "Delete" });
  await expectUntouched();
});

it("with no dialog, Delete on the page still removes the selected block", async () => {
  selectBlock("B");
  expect(press(document.body, { key: "Delete", code: "Delete" }).defaultPrevented).toBe(true);
  await flush();
  expect(node("B")).toBeFalsy();
});

it("a dialog that shows the selected block, as a hover preview does, leaves Delete to the selection", async () => {
  selectBlock("B");
  const preview = document.createElement("div");
  preview.setAttribute("role", "dialog");
  preview.tabIndex = -1;
  const row = document.createElement("div");
  row.dataset.blockId = "B";
  preview.append(row);
  document.body.append(preview);
  preview.focus();
  expect(press(preview, { key: "Delete", code: "Delete" }).defaultPrevented).toBe(true);
  await flush();
  expect(node("B")).toBeFalsy();
});

it("a hover preview showing over the page leaves Delete on the page to the page's selection", async () => {
  selectBlock("B");
  const preview = document.createElement("div");
  preview.setAttribute("role", "dialog");
  document.body.append(preview);
  disposers.push(registerTransientLayer({ id: "hover-preview", root: () => preview, dismiss: () => true }));
  expect(press(document.body, { key: "Delete", code: "Delete" }).defaultPrevented).toBe(true);
  await flush();
  expect(node("B")).toBeFalsy();
});

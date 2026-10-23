import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { clearTransientLayersForTest, dismissTopTransient } from "../transientLayers";
import { node, resetStore } from "../document";
import { editingId, endEdit, startEditing } from "../editorController";

// D-20261023-VQ: a hover preview shows the live source blocks, editable in
// place; while its editor holds the caret the popup stays put.
const OPEN_MS = 1000;

beforeAll(async () => { await initParser(); });
beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => {
  endEdit("blur");
  resetStore();
  clearTransientLayersForTest();
  vi.useRealTimers();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

async function advance(ms: number) {
  await vi.advanceTimersByTimeAsync(ms);
  await Promise.resolve();
  await Promise.resolve();
}
async function waitFor(fn: () => boolean, timeout = 1000): Promise<boolean> {
  for (let elapsed = 0; elapsed < timeout; elapsed += 25) {
    if (fn()) return true;
    await advance(25);
  }
  return fn();
}
const popup = () => document.body.querySelector<HTMLElement>(".peek-popup");
const editor = () => popup()?.querySelector<HTMLTextAreaElement>("textarea.block-editor") ?? null;

async function openPeek(): Promise<{ card: HTMLElement; dispose: () => void }> {
  const dto = {
    id: "pages/Probe.md", name: "Probe", title: "Probe", kind: "page", format: "md", pre_block: "",
    blocks: [{ id: "e1", raw: "Editable text", collapsed: false, children: [{ id: "e2", raw: "Child", collapsed: false, children: [] }] }],
  };
  vi.spyOn(backend(), "getPage").mockImplementation(async () => structuredClone(dto) as never);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async () => structuredClone(dto) as never);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <div class="block-content"><AstBody raw="see [[Probe]]" /></div>, host);
  const link = host.querySelector("a.page-ref")!;
  link.dispatchEvent(new PointerEvent("pointerenter", { bubbles: false, pointerType: "mouse", isPrimary: true }));
  link.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
  await advance(OPEN_MS);
  expect(await waitFor(() => !!popup()?.querySelector(".ls-block") && !!node("e1"))).toBe(true);
  return { card: popup()!, dispose: () => { dispose(); host.remove(); } };
}

async function editInPopup(card: HTMLElement) {
  card.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
  startEditing("e1", 0);
  expect(await waitFor(() => !!editor())).toBe(true);
  editor()!.focus();
  await advance(0);
}

describe("D-20261023-VQ: editable hover preview", () => {
  it("renders the live source block, and typing edits the loaded node", async () => {
    const { card, dispose } = await openPeek();
    try {
      // The live outline Block, not the read-only reference copy.
      expect(card.querySelector(".ls-block:not(.ref-block)")).not.toBeNull();
      await editInPopup(card);
      const textarea = editor()!;
      textarea.value = "Edited in preview";
      textarea.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await advance(0);
      expect(node("e1")!.raw).toBe("Edited in preview");
    } finally {
      dispose();
    }
  });

  it("stays open on pointer exit while editing, and resumes the grace period when the edit ends", async () => {
    const { card, dispose } = await openPeek();
    try {
      await editInPopup(card);
      card.dispatchEvent(new MouseEvent("mouseleave", { bubbles: false }));
      await advance(3000);
      expect(popup()).not.toBeNull();
      expect(editingId()).toBe("e1");
      endEdit("blur");
      expect(await waitFor(() => !popup(), 3000)).toBe(true);
    } finally {
      dispose();
    }
  });

  it("a fold in the preview is the source block's own fold (GH #748)", async () => {
    const { card, dispose } = await openPeek();
    try {
      card.querySelector<HTMLElement>(".ls-block:not(.ref-block) .collapse-toggle")!.click();
      await vi.waitFor(() => expect(node("e1")!.collapsed).toBe(true));
      expect(node("e1")!.raw).toContain("collapsed:: true");
    } finally {
      dispose();
    }
  });

  it("a plain bullet click opens the block from the preview instead of zooming the pane behind it", async () => {
    const { card, dispose } = await openPeek();
    try {
      const route = location.hash;
      card.querySelector<HTMLElement>(".ls-block:not(.ref-block) .bullet-container")!.click();
      await advance(0);
      expect(popup()).toBeNull(); // the preview's open-source action dismissed it
      expect(location.hash).toBe(route);
    } finally {
      dispose();
    }
  });

  it("ignores a page scroll while editing", async () => {
    const { card, dispose } = await openPeek();
    try {
      await editInPopup(card);
      document.dispatchEvent(new Event("scroll"));
      window.dispatchEvent(new Event("resize"));
      await advance(0);
      expect(popup()).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("the first Escape ends the edit, the second closes the popup", async () => {
    const { card, dispose } = await openPeek();
    try {
      await editInPopup(card);
      expect(dismissTopTransient("escape")).toBe(true);
      await advance(0);
      expect(editingId()).toBeNull();
      expect(popup()).not.toBeNull();
      expect(dismissTopTransient("escape")).toBe(true);
      await advance(0);
      expect(popup()).toBeNull();
    } finally {
      dispose();
    }
  });
});

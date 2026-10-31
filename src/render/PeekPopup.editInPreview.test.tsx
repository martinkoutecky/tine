import { maybeEditorIn } from "../tests/editorDriver";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { clearTransientLayersForTest, dismissTopTransient } from "../transientLayers";
import { deleteSelection, ensurePageLoaded, isDirty, isSelected, selectedIds, node, pageByName, resetStore } from "../document";
import { route, tabs } from "../router";
import { installKeybindings } from "../keybindings";
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
const editor = () => maybeEditorIn(popup()) ?? null;

const blk = (id: string, raw: string, children: unknown[] = [], extra: object = {}) => ({ id, raw, collapsed: false, children, ...extra });
const defaultBlocks = [blk("e1", "Editable text", [blk("e2", "Child")])];

async function openPeek(blocks: unknown[] = defaultBlocks, source: object = {}, live = true): Promise<{ card: HTMLElement; link: Element; dispose: () => void }> {
  const dto = {
    id: "pages/Probe.md", name: "Probe", title: "Probe", kind: "page", format: "md", pre_block: "", blocks, ...source,
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
  expect(await waitFor(() => !!popup()?.querySelector(".ls-block") && !!pageByName("Probe"))).toBe(true);
  if (live) expect(await waitFor(() => !!popup()?.querySelector(".ls-block:not(.ref-block)"))).toBe(true);
  return { card: popup()!, link, dispose: () => { dispose(); host.remove(); } };
}

async function editInPopup(card: HTMLElement, id = "e1") {
  card.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
  const content = card.querySelector<HTMLElement>(`.ls-block:not(.ref-block)[data-block-id="${id}"] .block-content`)!;
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  content.click(); // the user's own gesture, scoped to this rendered instance
  expect(await waitFor(() => !!editor())).toBe(true);
  editor()!.element.focus();
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
      textarea.replaceText("Edited in preview");
      textarea.reportInput({ bubbles: true });
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
      const before = tabs().length;
      card.querySelector<HTMLElement>('.ls-block:not(.ref-block)[data-block-id="e1"] .bullet-container')!.click();
      await advance(0);
      expect(popup()).toBeNull(); // the preview's open-source action dismissed it
      expect(tabs().length).toBe(before + 1);
      // Opened in the foreground, on the source block.
      expect(route()).toMatchObject({ kind: "page", name: "Probe", block: "e1" });
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
  it("a row whose id another loaded page owns stays read-only and never edits that page (duplicate id::)", async () => {
    const U = "58900000-0000-4000-8000-0000000000d1";
    ensurePageLoaded({ id: "pages/Orig.md", name: "Orig", title: "Orig", kind: "page", format: "md", pre_block: "",
      blocks: [blk(U, `Original\nid:: ${U}`, [], { properties: [["id", U]] })] } as never);
    const { card, dispose } = await openPeek([blk(U, `Copied\nid:: ${U}`, [], { properties: [["id", U]] })], {}, false);
    try {
      await advance(100);
      expect(card.textContent).toContain("Copied");
      expect(card.querySelector(".ls-block:not(.ref-block)")).toBeNull();
      expect(maybeEditorIn(card)).toBeNull();
      expect(node(U)!.page).toBe("Orig");
      expect(node(U)!.raw).toBe(`Original\nid:: ${U}`);
    } finally {
      dispose();
    }
  });

  it("a read-only source refuses editing and folds only its view after hydration", async () => {
    const { card, dispose } = await openPeek(defaultBlocks, { read_only: true }, false);
    try {
      await advance(100);
      expect(card.textContent).toContain("Child");
      const content = card.querySelector<HTMLElement>(".block-content")!;
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      content.click();
      await advance(0);
      expect(editor()).toBeNull();
      card.querySelector<HTMLElement>(".collapse-toggle")!.click();
      await vi.waitFor(() => expect(popup()!.textContent).not.toContain("Child"));
      expect(node("e1")!.collapsed).toBe(false);
      expect(node("e1")!.raw).toBe("Editable text");
      expect(isDirty("Probe")).toBe(false);
    } finally {
      dispose();
    }
  });

  it("an outside press while editing leaves the popup and its editor mounted", async () => {
    const { card, dispose } = await openPeek();
    try {
      await editInPopup(card);
      const outside = document.createElement("div");
      document.body.appendChild(outside);
      outside.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      outside.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      await advance(0);
      expect(popup()).not.toBeNull();
      expect(editor()).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("re-crossing the link while editing does not close the popup on the link's timer", async () => {
    const { card, link, dispose } = await openPeek();
    try {
      await editInPopup(card);
      card.dispatchEvent(new MouseEvent("mouseleave", { bubbles: false }));
      link.dispatchEvent(new PointerEvent("pointerenter", { bubbles: false, pointerType: "mouse", isPrimary: true }));
      link.dispatchEvent(new PointerEvent("pointerleave", { bubbles: false, pointerType: "mouse", isPrimary: true }));
      await advance(3000);
      expect(popup()).not.toBeNull();
      expect(editor()).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("ArrowDown navigates through the source folds the preview shows", async () => {
    const { card, dispose } = await openPeek([blk("r", "Root", [blk("c", "Child", [blk("g", "Grandchild")])]), blk("s", "Sibling")]);
    try {
      expect(card.textContent).toContain("Grandchild");
      await editInPopup(card, "c");
      const textarea = editor()!;
      textarea.select(textarea.value().length, textarea.value().length);
      textarea.keydown(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
      await advance(0);
      expect(editingId()).toBe("g");
    } finally {
      dispose();
    }
  });
  it("an unscoped edit elsewhere does not mount a second editor in the preview", async () => {
    const { dispose } = await openPeek();
    try {
      startEditing("e1", 0); // keyboard navigation or a split on the source page
      await advance(0);
      expect(editor()).toBeNull();
    } finally {
      dispose();
    }
  });
  it("selecting past an editable row never reaches another page's block behind a read-only duplicate row", async () => {
    const U = "58900000-0000-4000-8000-0000000000d2";
    ensurePageLoaded({ id: "pages/Orig.md", name: "Orig", title: "Orig", kind: "page", format: "md", pre_block: "",
      blocks: [blk(U, `Original\nid:: ${U}`, [], { properties: [["id", U]] })] } as never);
    const uninstall = installKeybindings(); // Shift+Down is a configured editor command
    const { card, dispose } = await openPeek([blk("e1", "Editable"), blk(U, `Copied\nid:: ${U}`, [], { properties: [["id", U]] })]);
    try {
      await editInPopup(card, "e1");
      const textarea = editor()!;
      textarea.select(textarea.value().length, textarea.value().length);
      textarea.keydown(new KeyboardEvent("keydown", { key: "ArrowDown", shiftKey: true, bubbles: true, cancelable: true }));
      await advance(0);
      textarea.keydown(new KeyboardEvent("keydown", { key: "ArrowDown", shiftKey: true, bubbles: true, cancelable: true }));
      await advance(0);
      expect(selectedIds()).toContain("e1"); // the block selection started
      expect(isSelected(U)).toBe(false);
      deleteSelection();
      await advance(0);
      expect(node(U)?.page).toBe("Orig");
      expect(node(U)?.raw).toBe(`Original\nid:: ${U}`);
    } finally {
      dispose();
      uninstall();
    }
  });
});

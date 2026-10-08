import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { editingId, editingSurface } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { doc } from "../document/model";
import type { PageRead } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";

// GH #341: arrow navigation out of an edited block inside a linked-reference /
// query / embed group must stay in that rendered surface and move to the
// adjacent RENDERED block. Display membership [rootA, rootB] is deliberately
// NOT the page's own root order [rootB, rootA]:
//   page order     : rootB, b1, rootA, a1, a2
//   rendered order : rootA, a1, a2, rootB, b1

const leaf = (id: string, raw: string) => ({ id, raw, collapsed: false, children: [] });

function pageDto(): PageRead {
  return {
    name: "Source",
    kind: "page",
    title: "Source",
    pre_block: null,
    id: "pages/Source.md",
    blocks: [
      { id: "rootB", raw: "Root B", collapsed: false, children: [leaf("b1", "Thing B")] },
      { id: "rootA", raw: "Root A", collapsed: false, children: [leaf("a1", "Thing 1"), leaf("a2", "Thing 2")] },
    ],
  };
}

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

function mockFiveCharacterVisualRows(): void {
  const width = 5;
  const lineHeight = 10;
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (this: HTMLElement) {
    return this instanceof HTMLDivElement ? lineHeight : 0;
  });
  vi.spyOn(HTMLElement.prototype, "offsetTop", "get").mockImplementation(function (this: HTMLElement) {
    if (!(this instanceof HTMLSpanElement)) return 0;
    const before = this.previousSibling?.textContent?.length ?? 0;
    const full = this.parentElement?.textContent?.replaceAll("\u200b", "") ?? "";
    const occupied = before === full.length && before > 0 ? before - 1 : before;
    return Math.floor(occupied / width) * lineHeight;
  });
}

async function settle(rounds = 6) {
  for (let i = 0; i < rounds; i++) await Promise.resolve();
}

function editorOf(root: HTMLElement, id: string): HTMLTextAreaElement | null {
  return root.querySelector(`.ls-block[data-block-id="${id}"] textarea.block-editor`);
}

async function mountAndEdit(id: string, surface: "ref" | "embed" = "ref") {
  const dto = pageDto();
  const [rootB, rootA] = dto.blocks;
  vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => (
    <LiveRefGroup
      page={dto.name}
      kind={dto.kind}
      blocks={[rootA, rootB]}
      surface={surface}
    />
  ), root);

  // Hydration first (the static RefBlocks fallback also carries
  // .block-content-wrapper — clicking IT does nothing), then the live block.
  await vi.waitFor(() => expect(doc.byId[id]).toBeTruthy());
  await vi.waitFor(() => {
    expect(root.querySelector(`.ls-block[data-block-id="${id}"] .block-content-wrapper`)).not.toBeNull();
  });

  const wrapper = root.querySelector<HTMLElement>(`.ls-block[data-block-id="${id}"] .block-content-wrapper`)!;
  wrapper.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: 0, clientY: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, clientX: 0, clientY: 0 }));
  await settle();
  const editor = editorOf(root, id);
  expect(editor).not.toBeNull();
  return { root, dispose, editor: editor! };
}

describe("arrow navigation inside ref/query/embed groups (GH #341)", () => {
  it("ArrowDown moves to the next rendered sibling inside the group", async () => {
    mockFiveCharacterVisualRows();
    const { root, editor, dispose } = await mountAndEdit("a1");
    editor.setSelectionRange(editor.value.length, editor.value.length);
    editor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
    await settle();

    expect(editingId()).toBe("a2");
    expect(editorOf(root, "a2")?.value).toBe("Thing 2");
    expect(editingSurface()?.startsWith("ref:")).toBe(true);
    expect(root.contains(document.activeElement)).toBe(true);
    dispose();
  });

  it("ArrowDown crosses display roots into the next rendered root", async () => {
    mockFiveCharacterVisualRows();
    const { root, editor, dispose } = await mountAndEdit("a2");
    editor.setSelectionRange(editor.value.length, editor.value.length);
    editor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
    await settle();

    // Page order ends after a2 (roots are [rootB, rootA]) — only the group's
    // RENDERED list (…a2, rootB…) can take this step.
    expect(editingId()).toBe("rootB");
    expect(editorOf(root, "rootB")?.value).toBe("Root B");
    expect(root.contains(document.activeElement)).toBe(true);
    dispose();
  });

  it("ArrowUp moves to the previous rendered block, not the page-order sibling", async () => {
    mockFiveCharacterVisualRows();
    const { root, editor, dispose } = await mountAndEdit("rootB");
    editor.setSelectionRange(1, 1);
    editor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true, cancelable: true }));
    await settle();

    // Rendered order: a2 precedes rootB. Page order would answer b1 — an
    // editor that is not adjacent in this view (the issue's caret thief).
    expect(editingId()).toBe("a2");
    expect(editorOf(root, "a2")?.value).toBe("Thing 2");
    dispose();
  });

  it("embed groups get the same rendered-order navigation", async () => {
    mockFiveCharacterVisualRows();
    const { root, editor, dispose } = await mountAndEdit("a2", "embed");
    editor.setSelectionRange(editor.value.length, editor.value.length);
    editor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
    await settle();

    expect(editingId()).toBe("rootB");
    expect(editorOf(root, "rootB")?.value).toBe("Root B");
    expect(editingSurface()?.startsWith("embed:")).toBe(true);
    dispose();
  });

  it("does not bind a structural destination to a result-only surface", async () => {
    mockFiveCharacterVisualRows();
    const { editor, dispose } = await mountAndEdit("rootB");
    editor.setSelectionRange(2, 2);
    editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await settle();

    // Arrow destinations belong to this rendered list. A split destination is
    // source-outline structure and may no longer be a result, so it must not be
    // constrained to the ref surface where no matching editor may mount.
    expect(editingId()).not.toBe("rootB");
    expect(editingSurface()).toBeNull();
    dispose();
  });

  it("Backspace at the start of a first display root does not merge into the rendered neighbor", async () => {
    mockFiveCharacterVisualRows();
    const { root, editor, dispose } = await mountAndEdit("rootB");
    editor.setSelectionRange(0, 0);
    editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Backspace", bubbles: true, cancelable: true }));
    await settle();

    // Page order has nothing before rootB; nothing may merge — least of all
    // a2, which merely sits above rootB in this display list.
    expect(doc.byId.rootB?.raw).toBe("Root B");
    expect(doc.byId.a2?.raw).toBe("Thing 2");
    expect(pageByName("Source")!.roots).toEqual(["rootB", "rootA"]);
    expect(editingId()).toBe("rootB");
    expect(editorOf(root, "rootB")).not.toBeNull();
    dispose();
  });
});

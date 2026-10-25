import { editorIn, maybeEditorIn } from "../tests/editorDriver";
import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ensurePageLoaded, isSelected, node, resetStore, selectedIds, setBlockProperty } from "../document";
import { editingId, endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";
import type { BlockDto, PageDto } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";
import { ReferenceExcerptBlocks } from "./ReferenceEvidence";
import { contextMenu, closeContextMenu } from "../ui";
import { blockRefTarget } from "../resolveBatch";

const U = "58900000-0000-4000-8000-0000000000d3";
const block = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const copy = (body: string) => ({ ...block(U, `${body}\nid:: ${U}`), properties: [["id", U]] as [string, string][] });
const page = (name: string, blocks: BlockDto[], extra: Partial<PageDto> = {}): PageDto & { id: string } => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", format: "md", pre_block: "", blocks, ...extra,
});
afterEach(() => {
  endEdit("blur");
  closeContextMenu();
  resetStore();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});
function mount(view: Parameters<typeof render>[0]) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(view, root) };
}

describe("reference row source identity", () => {
  it.each(["ref", "embed", "preview", "query"] as const)("%s shows a copied row without exposing the other page's live block", async (surface) => {
    ensurePageLoaded(page("A", [copy("Original")]));
    const rows = [copy("Copied")];
    ensurePageLoaded(page("B", rows));
    if (surface === "query") adoptResults(rows, captureResultsTicket());
    const { root, dispose } = mount(() => <LiveRefGroup page="B" kind="page" path="pages/B.md" blocks={rows} surface={surface} eager />);
    try {
      await expect.poll(() => root.textContent).toContain("Copied");
      expect(root.textContent).not.toContain("Original");
      expect(root.querySelector(`.ls-block:not(.ref-block)[data-block-id="${U}"]`)).toBeNull();
      root.querySelector<HTMLElement>(".block-content")!.click();
      expect(maybeEditorIn(root)).toBeNull();
      expect(node(U)?.raw).toBe(`Original\nid:: ${U}`);
    } finally { dispose(); }
  });

  it.each(["ref", "embed"] as const)("%s navigation and selection exclude a copied read-only row", async (surface) => {
    ensurePageLoaded(page("A", [copy("Original")]));
    const rows = [block("editable", "Editable"), copy("Copied")];
    ensurePageLoaded(page("B", rows));
    const uninstall = installKeybindings();
    const { root, dispose } = mount(() => <LiveRefGroup page="B" kind="page" blocks={rows} surface={surface} eager />);
    try {
      await expect.poll(() => root.querySelector('.ls-block:not(.ref-block)[data-block-id="editable"]')).not.toBeNull();
      const content = root.querySelector<HTMLElement>('[data-block-id="editable"] .block-content')!;
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      content.click();
      await expect.poll(() => maybeEditorIn(root)).not.toBeNull();
      const editor = editorIn(root);
      editor.focus();
      editor.select(editor.value().length, editor.value().length);
      for (let i = 0; i < 2; i++) editor.keydown({ key: "ArrowDown", shiftKey: true, bubbles: true, cancelable: true });
      expect(selectedIds()).toContain("editable");
      expect(isSelected(U)).toBe(false);
      expect(editingId()).not.toBe(U);
    } finally { dispose(); uninstall(); }
  });

  it.each(["ref", "embed"] as const)("%s still edits an ordinary row on its source page", async (surface) => {
    const rows = [block("owned", "Source text")];
    ensurePageLoaded(page("B", rows));
    const { root, dispose } = mount(() => <LiveRefGroup page="B" kind="page" blocks={rows} surface={surface} eager />);
    try {
      await expect.poll(() => root.querySelector(".ls-block:not(.ref-block)")).not.toBeNull();
      const content = root.querySelector<HTMLElement>(".block-content")!;
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      content.click();
      await expect.poll(() => maybeEditorIn(root)).not.toBeNull();
      const editor = editorIn(root);
      editor.replaceText("Edited source");
      editor.reportInput({ bubbles: true });
      expect(node("owned")?.raw).toBe("Edited source");
    } finally { dispose(); }
  });

  it("an unlinked excerpt menu never targets the block on another page", () => {
    ensurePageLoaded(page("A", [copy("Original")]));
    const { root, dispose } = mount(() => <ReferenceExcerptBlocks page="B" kind="page" blocks={[copy("Copied")]} evidence={[]} />);
    try {
      root.querySelector(".reference-excerpt-row")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
      expect(contextMenu()).toBeNull();
    } finally { dispose(); }
  });

  it("source navigation keeps the backend row's page when another page owns its UUID", () => {
    ensurePageLoaded(page("A", [copy("Original")]));
    expect(blockRefTarget(U, { page: "B", kind: "page", path: "pages/B.md", blocks: [copy("Copied")] }))
      .toEqual({ uuid: U, page: "B", pageKind: "page", path: "pages/B.md" });
  });

  it.each(["ref", "embed"] as const)("%s keeps its live row when Copy block ref stamps an authored identity", async (surface) => {
    const rows = [block("owned", "Source text")];
    ensurePageLoaded(page("B", rows));
    const { root, dispose } = mount(() => <LiveRefGroup page="B" kind="page" blocks={rows} surface={surface} eager />);
    try {
      await expect.poll(() => root.querySelector(".ls-block:not(.ref-block)")).not.toBeNull();
      // Copy block ref uses the existing live key for the rest of the session.
      setBlockProperty("owned", "id", U);
      expect(root.querySelector(".ls-block:not(.ref-block)")).not.toBeNull();
      expect(root.textContent).toContain("Source text");
    } finally { dispose(); }
  });

  it.each(["ref", "embed"] as const)("%s refuses a page/journal twin or a different physical source", async (surface) => {
    const rows = [{ ...block("owned", "TODO Source text"), marker: "TODO" }];
    ensurePageLoaded(page("B", rows));
    for (const descriptor of [{ kind: "journal" as const }, { kind: "page" as const, path: "pages/other.md" }]) {
      const { root, dispose } = mount(() => <LiveRefGroup page="B" {...descriptor} blocks={rows} surface={surface} eager />);
      try {
        await expect.poll(() => root.textContent).toContain("Source text");
        expect(root.querySelector(".ls-block:not(.ref-block)")).toBeNull();
        expect(root.querySelector(".block-task-checkbox")).toBeNull();
      } finally { dispose(); }
    }
  });
});

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { editingId, endEdit } from "../editorController";
import { initParser } from "../render/parse";
import { doc, loadSingle, resetStore, undo } from "../store";
import type { BlockDto, PageDto, RefGroup } from "../types";
import { Block } from "./Block";
import { LiveRefGroup } from "./LiveRefGroup";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

function fixture(targetId: string): { page: PageDto; group: RefGroup } {
  const grandchild: BlockDto = {
    id: `${targetId}-grandchild`, raw: "Embedded grandchild", collapsed: false, children: [],
  };
  const child: BlockDto = {
    id: `${targetId}-child`, raw: "Embedded child", collapsed: false, children: [grandchild],
  };
  const target: BlockDto = {
    id: targetId,
    raw: `Embedded parent\nid:: ${targetId}`,
    collapsed: false,
    children: [child],
  };
  const page: PageDto = {
    name: "Block Embed Test",
    kind: "page",
    title: "Block Embed Test",
    pre_block: null,
    blocks: [target, { id: `${targetId}-host`, raw: `{{embed ((${targetId}))}}`, collapsed: false, children: [] }],
  };
  return { page, group: { page: page.name, kind: page.kind, blocks: [target] } };
}

function renderFixture(targetId: string) {
  const { page, group } = fixture(targetId);
  vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([group]);
  loadSingle(page);
  const hostId = `${targetId}-host`;
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(
    () => (
      <>
        <div class="main-source"><Block id={targetId} /></div>
        <Block id={hostId} />
      </>
    ),
    root,
  );
  return { root, dispose, hostId };
}

function mouseDownAndUp(element: Element): void {
  element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
}

describe("block embed hierarchy", () => {
  it("uses only the embedded root's interactive bullet", async () => {
    const targetId = "embed-hierarchy-root";
    const { root, dispose, hostId } = renderFixture(targetId);

    try {
      const host = root.querySelector<HTMLElement>(`[data-block-id="${hostId}"]`);
      expect(host).not.toBeNull();
      await vi.waitFor(() => expect(host?.querySelectorAll(".bullet-container")).toHaveLength(3));
      expect(host?.classList.contains("block-embed-host")).toBe(true);
      expect(host?.querySelector(".embed-block .bullet-container")).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("runs an embedded disclosure control through the real pointer sequence without editing the host", async () => {
    const targetId = "embed-pointer-collapse";
    const { root, dispose } = renderFixture(targetId);

    try {
      const toggle = await vi.waitFor(() => {
        const element = root.querySelector<HTMLElement>(
          `.embed-block [data-block-id="${targetId}"] > .block-main .collapse-toggle.has-children`,
        );
        expect(element).not.toBeNull();
        return element!;
      });

      mouseDownAndUp(toggle);

      // A real browser only sends click if the mousedown/mouseup target survives.
      // The regression entered the macro-host editor on mouseup and removed this
      // control before its click handler could run.
      expect(toggle.isConnected).toBe(true);
      expect(editingId()).toBeNull();
      toggle.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0 }));

      await vi.waitFor(() => expect(root.querySelector(".embed-block")?.textContent).not.toContain("Embedded child"));
      // Logseq keeps collapse state local to a reference/embed container; folding
      // the embed must not write collapsed:: onto the source block.
      expect(doc.byId[targetId].collapsed).toBe(false);
      const expand = root.querySelector<HTMLElement>(
        `.embed-block [data-block-id="${targetId}"] > .block-main .collapse-toggle.has-children`,
      );
      expect(expand).not.toBeNull();
      mouseDownAndUp(expand!);
      expect(expand!.isConnected).toBe(true);
      expand!.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0 }));
      await vi.waitFor(() => expect(root.querySelector(".embed-block")?.textContent).toContain("Embedded child"));
    } finally {
      dispose();
    }
  });

  it("keeps recursive guide collapse local to the embed surface", async () => {
    const targetId = "embed-guide-collapse";
    const { root, dispose } = renderFixture(targetId);
    const childId = `${targetId}-child`;

    try {
      const guide = await vi.waitFor(() => {
        const element = root.querySelector<HTMLButtonElement>(
          `.embed-block [data-block-id="${targetId}"] > .block-children-container > button.block-children-left-border`,
        );
        expect(element).not.toBeNull();
        return element!;
      });
      guide.click();

      await vi.waitFor(() =>
        expect(root.querySelector(".embed-block")?.textContent).not.toContain("Embedded grandchild")
      );
      expect(doc.byId[childId].collapsed).toBe(false);
      expect(doc.byId[childId].raw).not.toContain("collapsed:: true");

      guide.click();
      await vi.waitFor(() =>
        expect(root.querySelector(".embed-block")?.textContent).toContain("Embedded grandchild")
      );
      expect(doc.byId[childId].collapsed).toBe(false);
    } finally {
      dispose();
    }
  });

  it("keeps recursive guide collapse local to a reference surface", async () => {
    const targetId = "reference-guide-collapse";
    const { page, group } = fixture(targetId);
    loadSingle(page);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(
      () => <LiveRefGroup page={page.name} kind={page.kind} blocks={group.blocks} surface="ref" />,
      root,
    );
    try {
      const guide = await vi.waitFor(() => {
        const element = root.querySelector<HTMLButtonElement>(
          `[data-block-id="${targetId}"] > .block-children-container > button.block-children-left-border`,
        );
        expect(element).not.toBeNull();
        return element!;
      });
      guide.click();
      await vi.waitFor(() => expect(root.textContent).not.toContain("Embedded grandchild"));
      expect(doc.byId[`${targetId}-child`].collapsed).toBe(false);
      expect(doc.byId[`${targetId}-child`].raw).not.toContain("collapsed:: true");
    } finally {
      dispose();
    }
  });

  it("keeps Enter-created blocks and the caret in the visible embed surface", async () => {
    const targetId = "embed-enter-surface";
    const { root, dispose } = renderFixture(targetId);

    try {
      const embeddedContent = await vi.waitFor(() => {
        const element = root.querySelector<HTMLElement>(
          `.embed-block [data-block-id="${targetId}"] > .block-main .block-content`,
        );
        expect(element).not.toBeNull();
        return element!;
      });
      mouseDownAndUp(embeddedContent);

      const embeddedEditor = await vi.waitFor(() => {
        const editor = root.querySelector<HTMLTextAreaElement>(
          `.embed-block [data-block-id="${targetId}"] textarea.block-editor`,
        );
        expect(editor).not.toBeNull();
        return editor!;
      });
      embeddedEditor.setSelectionRange(embeddedEditor.value.length, embeddedEditor.value.length);
      embeddedEditor.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      const createdId = editingId();
      expect(createdId).not.toBeNull();
      expect(createdId).not.toBe(targetId);
      await vi.waitFor(() => {
        const active = document.activeElement;
        expect(active).toBeInstanceOf(HTMLTextAreaElement);
        expect(active?.closest(".embed-block")).not.toBeNull();
        expect(active?.closest(`[data-block-id="${createdId}"]`)).not.toBeNull();
      });
      expect(doc.byId[createdId!].page).toBe("Block Embed Test");
      undo();
      expect(doc.byId[createdId!]).toBeUndefined();
    } finally {
      dispose();
    }
  });

  it("keeps ArrowUp and ArrowDown navigation in the visible embed surface", async () => {
    const targetId = "embed-arrow-surface";
    const childId = `${targetId}-child`;
    const { root, dispose } = renderFixture(targetId);

    try {
      const embeddedChild = await vi.waitFor(() => {
        const element = root.querySelector<HTMLElement>(
          `.embed-block [data-block-id="${childId}"] > .block-main .block-content`,
        );
        expect(element).not.toBeNull();
        return element!;
      });
      mouseDownAndUp(embeddedChild);

      const childEditor = await vi.waitFor(() => {
        const editor = root.querySelector<HTMLTextAreaElement>(
          `.embed-block [data-block-id="${childId}"] textarea.block-editor`,
        );
        expect(editor).not.toBeNull();
        return editor!;
      });
      childEditor.setSelectionRange(0, 0);
      childEditor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));

      const parentEditor = await vi.waitFor(() => {
        const editor = root.querySelector<HTMLTextAreaElement>(
          `.embed-block [data-block-id="${targetId}"] textarea.block-editor`,
        );
        expect(editor).not.toBeNull();
        expect(document.activeElement).toBe(editor);
        return editor!;
      });
      expect(editingId()).toBe(targetId);

      parentEditor.setSelectionRange(parentEditor.value.length, parentEditor.value.length);
      parentEditor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      await vi.waitFor(() => {
        const editor = root.querySelector<HTMLTextAreaElement>(
          `.embed-block [data-block-id="${childId}"] textarea.block-editor`,
        );
        expect(editor).not.toBeNull();
        expect(document.activeElement).toBe(editor);
      });
      expect(editingId()).toBe(childId);
    } finally {
      dispose();
    }
  });

  it("keeps the empty editor and caret in the visible embed after deleting all text", async () => {
    const targetId = "embed-delete-surface";
    const childId = `${targetId}-child`;
    const { root, dispose } = renderFixture(targetId);

    try {
      const embeddedChild = await vi.waitFor(() => {
        const element = root.querySelector<HTMLElement>(
          `.embed-block [data-block-id="${childId}"] > .block-main .block-content`,
        );
        expect(element).not.toBeNull();
        return element!;
      });
      mouseDownAndUp(embeddedChild);

      const editor = await vi.waitFor(() => {
        const element = root.querySelector<HTMLTextAreaElement>(
          `.embed-block [data-block-id="${childId}"] textarea.block-editor`,
        );
        expect(element).not.toBeNull();
        return element!;
      });
      editor.setSelectionRange(0, editor.value.length);
      editor.value = "";
      editor.dispatchEvent(new InputEvent("input", {
        bubbles: true,
        inputType: "deleteContentForward",
      }));

      await vi.waitFor(() => {
        expect(doc.byId[childId].raw).toBe("");
        expect(editingId()).toBe(childId);
        expect(document.activeElement).toBe(editor);
        expect(editor.closest(".embed-block")).not.toBeNull();
      });
    } finally {
      dispose();
    }
  });
});

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { editingId, endEdit } from "../editorController";
import { dispatchFocusedEditorCommand } from "../editorCommandBridge";
import { initParser } from "../render/parse";
import { pageByName, resetStore, selectedIds, node, indentSelection, outdentSelection } from "../document";
import { loadFeed, loadSingle } from "../document/workingSet";
import type { BlockDto, PageDto, RefGroup } from "../types";
import { Block } from "./Block";
import { QuickSwitcher } from "./QuickSwitcher";
import { closeSwitcher, openCommandPalette } from "../ui";

// GH #415: arrow navigation INSIDE a block embed stays surface-local (GH #341
// contract), but Up from the FIRST visual row of the embed ROOT has no
// embed-internal destination — it exits to the block preceding the embed in
// the host page, the same block the whole embed visually sits under. Before
// this fix the caret was trapped: nothing happened at the top of an embed.

class AllNearObserver implements IntersectionObserver {
  readonly root = null;
  readonly rootMargin = "0px";
  readonly thresholds = [0];
  constructor(private readonly callback: IntersectionObserverCallback) {}
  disconnect(): void {}
  takeRecords(): IntersectionObserverEntry[] { return []; }
  unobserve(): void {}
  observe(target: Element): void {
    this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this);
  }
}

beforeAll(async () => {
  await initParser();
});

beforeEach(() => {
  vi.stubGlobal("IntersectionObserver", AllNearObserver);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  endEdit("page-navigation");
  closeSwitcher();
  resetStore();
  document.body.innerHTML = "";
});

function leaf(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  return { id, raw, collapsed: false, children };
}

// Host page order: "above" → host ({{embed ((target))}}) → target. The block
// above the embed is deliberately neither the host nor the embedded target,
// so the exit destination is unambiguous.
function loadFixture() {
  const above = leaf("above", "plain above");
  const host = leaf("host", "{{embed ((target))}}");
  const target = leaf("target", "embedded root", [leaf("target-child", "embedded child")]);
  const page: PageDto = { name: "HostPage", kind: "page", title: "HostPage", pre_block: null, blocks: [above, host, target] };
  const group: RefGroup = { page: page.name, kind: page.kind, blocks: [{ ...target, children: [] }] };
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) =>
    ids.map((id) => (id === "target" ? group : null))
  );
  loadSingle(page);
  return page;
}

function mouseDownAndUp(element: Element): void {
  element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
}

describe("Up arrow at the top of an embedded block (GH #415)", () => {
  it("Select embedding block runs from the real command palette after focus leaves the source", async () => {
    loadFixture();
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <><For each={pageByName("HostPage")?.roots ?? []}>{id => <Block id={id}/>}</For><QuickSwitcher/></>, root);
    try {
      const content = await vi.waitFor(() => {
        const el = root.querySelector<HTMLElement>('.embed-block [data-block-id="target-child"] .block-content');
        expect(el).not.toBeNull(); return el!;
      });
      mouseDownAndUp(content);
      await vi.waitFor(() => expect(editingId()).toBe("target-child"));
      openCommandPalette();
      await vi.waitFor(() => expect(document.activeElement?.classList.contains("switcher-input")).toBe(true));
      const command = [...root.querySelectorAll<HTMLElement>(".switcher-row")].find(el => el.textContent?.includes("Select embedding block"));
      expect(command).toBeDefined();
      command!.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(selectedIds()).toEqual(["host"]));
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });
  it("selects the embedding block from a nested embedded source (GH #641)", async () => {
    loadFixture();
    const root = document.createElement("div"); document.body.appendChild(root);
    const dispose = render(() => <For each={pageByName("HostPage")?.roots ?? []}>{id => <Block id={id} />}</For>, root);
    try {
      const content = await vi.waitFor(() => {
        const el = root.querySelector<HTMLElement>('.embed-block [data-block-id="target-child"] .block-content');
        expect(el).not.toBeNull(); return el!;
      });
      mouseDownAndUp(content);
      await vi.waitFor(() => expect(editingId()).toBe("target-child"));
      expect(dispatchFocusedEditorCommand("editor/select-embedding-block")).toBe(true);
      expect(editingId()).toBeNull();
      expect(selectedIds()).toEqual(["host"]);
      expect(root.querySelector(".embed-block")).not.toBeNull();
      indentSelection();
      expect(node("host").parent).toBe("above");
      expect(node("target-child").parent).toBe("target");
      outdentSelection();
      expect(node("host").parent).toBeNull();
    } finally { dispose(); }
  });
  it("selects the containing macro from a page embed too", async () => {
    const source: PageDto = { name: "SourcePage", title: "SourcePage", kind: "page", pre_block: null, blocks: [leaf("source", "source text", [leaf("kid", "kid text")])] };
    vi.spyOn(backend(), "getPage").mockResolvedValue({ ...source, id: "pages/SourcePage.md" });
    loadFeed([{ name: "HostPage", title: "HostPage", kind: "page", pre_block: null, blocks: [leaf("host", "{{embed [[SourcePage]]}}"), leaf("below", "below")] }, source]);
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <For each={pageByName("HostPage")?.roots ?? []}>{id => <Block id={id}/>}</For>, root);
    try {
      const content = await vi.waitFor(() => {
        const el = root.querySelector<HTMLElement>('.embed-block [data-block-id="kid"] .block-content');
        expect(el).not.toBeNull(); return el!;
      });
      mouseDownAndUp(content);
      await vi.waitFor(() => expect(editingId()).toBe("kid"));
      expect(dispatchFocusedEditorCommand("editor/select-embedding-block")).toBe(true);
      expect(selectedIds()).toEqual(["host"]);
      expect(editingId()).toBeNull();
    } finally { dispose(); }
  });
  it("leaves the embed and lands on the preceding block of the host page, keeping the embed rendered", async () => {
    loadFixture();
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => (
      <For each={pageByName("HostPage")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ), root);

    try {
      const embeddedContent = await vi.waitFor(() => {
        const element = root.querySelector<HTMLElement>(
          `.embed-block [data-block-id="target"] > .block-main .block-content`,
        );
        expect(element).not.toBeNull();
        return element!;
      });
      mouseDownAndUp(embeddedContent);

      const embeddedEditor = await vi.waitFor(() => {
        const editor = root.querySelector<HTMLTextAreaElement>(
          `.embed-block [data-block-id="target"] textarea.block-editor`,
        );
        expect(editor).not.toBeNull();
        return editor!;
      });
      // Caret at the very start of the embed root's first (single-line) row.
      embeddedEditor.setSelectionRange(0, 0);
      embeddedEditor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true, cancelable: true }));

      await vi.waitFor(() => expect(editingId()).toBe("above"));
      // The exit mounts in the host page's primary surface — NOT back inside
      // the embed, and NOT into the raw macro text of the embedding block.
      await vi.waitFor(() => {
        const active = document.activeElement;
        expect(active).toBeInstanceOf(HTMLTextAreaElement);
        expect((active as HTMLTextAreaElement).value).toBe("plain above");
        expect(active!.closest(".embed-block")).toBeNull();
      });
      // The embed stays rendered (only its root lost the caret); internal
      // navigation remains surface-local by contract.
      expect(root.querySelector(".embed-block")).not.toBeNull();
      expect(editingId()).not.toBe("host");
    } finally {
      dispose();
    }
  });

  it("Up from the first row of a NON-root embedded row still stays inside the embed", async () => {
    loadFixture();
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => (
      <For each={pageByName("HostPage")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ), root);

    try {
      const childContent = await vi.waitFor(() => {
        const element = root.querySelector<HTMLElement>(
          `.embed-block [data-block-id="target-child"] > .block-main .block-content`,
        );
        expect(element).not.toBeNull();
        return element!;
      });
      mouseDownAndUp(childContent);

      const childEditor = await vi.waitFor(() => {
        const editor = root.querySelector<HTMLTextAreaElement>(
          `.embed-block [data-block-id="target-child"] textarea.block-editor`,
        );
        expect(editor).not.toBeNull();
        return editor!;
      });
      childEditor.setSelectionRange(0, 0);
      childEditor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true, cancelable: true }));

      // Surface-local internal navigation (GH #341) is unchanged: the child
      // climbs to the embed root INSIDE the embed.
      await vi.waitFor(() => expect(editingId()).toBe("target"));
      expect(document.activeElement?.closest(".embed-block")).not.toBeNull();
    } finally {
      dispose();
    }
  });
});

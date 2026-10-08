import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { editingId, endEdit, startEditing } from "../editorController";
import { clearSeededFacets } from "../render/facets";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";
import { resetStore, undo } from "../document";
import { pageToDto } from "../document/convert";
import { loadSingle, loadFeed } from "../document/workingSet";
import { doc, pageByName } from "../document/model";
import type { BlockDto, PageDto, RefGroup } from "../types";
import { Block } from "./Block";

// GH #477: a structural edit made INSIDE a block embed must leave the caret in
// the embed. Tab/Shift+Tab and Alt+Shift+Up/Down went through store operations
// that called `startEditing` without naming a surface; with no surface named,
// `editing()` (Block.tsx) deliberately prefers the NON-embed rendering, so the
// editor remounted on the source copy of the block further down the page and
// the user's caret jumped out of the embed mid-keystroke.
//
// The fixture renders the target block TWICE on purpose — once inside the
// embed, once as an ordinary root of the host page. That is the whole bug: with
// only one rendering there is no wrong surface to land on.

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

describe("Enter after a collapsed embed host (GH #642)", () => {
  it.each([false, true])("creates a host sibling from the collapsed block occurrence; terminal=%s", async terminal => {
    loadFixture();
    // Occurrence-local collapse: the source stays expanded and unchanged.
    const host = leaf("host", "{{embed ((target))}}\ncollapsed:: true");
    const source = pageToDto("HostPage")!.blocks[1];
    loadSingle({ name: "HostPage", title: "HostPage", kind: "page", pre_block: null,
      blocks: terminal ? [source, host] : [host, source] });
    clearSeededFacets();
    const before = pageToDto("HostPage");
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <For each={pageByName("HostPage")!.roots}>{id => <Block id={id} />}</For>, root);
    try {
      const content = await vi.waitFor(() => {
        const element = root.querySelector('.embed-block [data-block-id="target"] > .block-main .block-content');
        expect(element).not.toBeNull(); return element!;
      });
      mouseDownAndUp(content);
      const editor = await vi.waitFor(() => {
        const element = root.querySelector<HTMLTextAreaElement>('.embed-block textarea.block-editor');
        expect(element).not.toBeNull(); return element!;
      });
      editor.setSelectionRange(editor.value.length, editor.value.length);
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      const roots = pageByName("HostPage")!.roots;
      expect(roots).toHaveLength(3);
      const newId = roots[roots.indexOf("host") + 1];
      expect(newId).not.toBe("target");
      expect(doc.byId[newId].raw).toBe("");
      expect(doc.byId[newId].parent).toBeNull();
      expect(editingId()).toBe(newId);
      expect(pageToDto("HostPage")!.blocks.find(b => b.id === "target")).toEqual(source);
      undo();
      expect(pageToDto("HostPage")).toEqual(before);
    } finally { dispose(); }
  });

  it.each([false, true])("Enter in a collapsed page-embed host creates a sibling; terminal=%s", terminal => {
    const host = { ...leaf("host", "{{embed [[SourcePage]]}}"), collapsed: true };
    const after = leaf("after", "after");
    const source = { id: "source-page", name: "SourcePage", title: "SourcePage", kind: "page" as const, pre_block: null,
      blocks: [leaf("source", "source text", [leaf("child", "child text")])] };
    vi.spyOn(backend(), "getPage").mockResolvedValue(source);
    loadFeed([{ name: "HostPage", title: "HostPage", kind: "page", pre_block: null,
      blocks: terminal ? [host] : [host, after] }, source]);
    const sourceBefore = pageToDto("SourcePage");
    const before = pageToDto("HostPage");
    startEditing("host", host.raw.length);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <For each={pageByName("HostPage")!.roots}>{id => <Block id={id} />}</For>, root);
    try {
      const editor = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      editor.setSelectionRange(editor.value.length, editor.value.length);
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      const roots = pageByName("HostPage")!.roots;
      expect(roots).toHaveLength(terminal ? 2 : 3);
      expect(doc.byId[roots[1]].raw).toBe("");
      expect(doc.byId[roots[1]].parent).toBeNull();
      expect(pageToDto("SourcePage")).toEqual(sourceBefore);
      undo(); expect(pageToDto("HostPage")).toEqual(before);
      expect(pageToDto("SourcePage")).toEqual(sourceBefore);
    } finally { dispose(); }
  });

  it("Enter genuinely inside the embedded source keeps editing the source", async () => {
    await withHostPage(async root => {
      const editor = await editInsideEmbed(root);
      editor.setSelectionRange(editor.value.length, editor.value.length);
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      expect(pageByName("HostPage")!.roots).toEqual(["host", "target"]);
      expect(doc.byId.target.children).toHaveLength(3);
    });
  });

  it("Enter in a folded source root of a page embed stays source-scoped", async () => {
    const host = leaf("host", "{{embed [[SourcePage]]}}");
    const source = { id: "source-page", name: "SourcePage", title: "SourcePage", kind: "page" as const, pre_block: null,
      blocks: [{ ...leaf("source", "source text", [leaf("child", "child text")]), collapsed: true }] };
    vi.spyOn(backend(), "getPage").mockResolvedValue(source);
    loadFeed([{ name: "HostPage", title: "HostPage", kind: "page", pre_block: null, blocks: [host] }, source]);
    const root = document.createElement("div"); document.body.appendChild(root);
    const dispose = render(() => <Block id="host" />, root);
    try {
      const content = await vi.waitFor(() => {
        const element = root.querySelector('.embed-block [data-block-id="source"] > .block-main .block-content');
        expect(element).not.toBeNull(); return element!;
      });
      mouseDownAndUp(content);
      const editor = await vi.waitFor(() => {
        const element = root.querySelector<HTMLTextAreaElement>('.embed-block textarea.block-editor');
        expect(element).not.toBeNull(); return element!;
      });
      editor.setSelectionRange(editor.value.length, editor.value.length);
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      expect(pageByName("HostPage")!.roots).toEqual(["host"]);
      expect(pageByName("SourcePage")!.roots).toHaveLength(2);
      expect(doc.byId.source.children).toEqual(["child"]);
      undo(); expect(pageToDto("SourcePage")!.blocks).toEqual(source.blocks);
    } finally { dispose(); }
  });
});

// Tab and Alt+Shift+Up reach the editor through the configurable binding table,
// which is empty until the keymap is installed.
let disposeKeys: (() => void) | null = null;

beforeEach(() => {
  vi.stubGlobal("IntersectionObserver", AllNearObserver);
  disposeKeys = installKeybindings();
});

afterEach(() => {
  disposeKeys?.();
  disposeKeys = null;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

function leaf(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  return { id, raw, collapsed: false, children };
}

// Two children under the embedded root, so the SECOND can be indented under the
// first (Tab needs a previous sibling) and outdented back again.
function loadFixture() {
  const target = leaf("target", "embedded root", [
    leaf("kid-one", "child one"),
    leaf("kid-two", "child two"),
  ]);
  const host = leaf("host", "{{embed ((target))}}");
  const page: PageDto = {
    name: "HostPage",
    kind: "page",
    title: "HostPage",
    pre_block: null,
    blocks: [host, target],
  };
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

/** Mount the host page and put the caret in `kid-two` INSIDE the embed. */
async function editInsideEmbed(root: HTMLElement) {
  const content = await vi.waitFor(() => {
    const element = root.querySelector<HTMLElement>(
      `.embed-block [data-block-id="kid-two"] > .block-main .block-content`,
    );
    expect(element).not.toBeNull();
    return element!;
  });
  mouseDownAndUp(content);
  const editor = await vi.waitFor(() => {
    const element = root.querySelector<HTMLTextAreaElement>(
      `.embed-block [data-block-id="kid-two"] textarea.block-editor`,
    );
    expect(element).not.toBeNull();
    return element!;
  });
  editor.setSelectionRange(0, 0);
  return editor;
}

/** The caret is on the embedded copy, not the source copy further down. */
async function expectCaretStayedInTheEmbed() {
  await vi.waitFor(() => {
    const active = document.activeElement;
    expect(active).toBeInstanceOf(HTMLTextAreaElement);
    expect((active as HTMLTextAreaElement).value).toBe("child two");
    expect(active!.closest(".embed-block")).not.toBeNull();
  });
}

async function withHostPage(body: (root: HTMLElement) => Promise<void>) {
  loadFixture();
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => (
    <For each={pageByName("HostPage")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  try {
    await body(root);
  } finally {
    dispose();
  }
}

describe("a structural edit inside a block embed keeps the caret there (GH #477)", () => {
  it("keeps Tab (indent) in the embed", async () => {
    await withHostPage(async (root) => {
      const editor = await editInsideEmbed(root);
      editor.setSelectionRange(2, 7, "backward");
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(doc.byId["kid-two"]?.parent).toBe("kid-one"));
      expect(editingId()).toBe("kid-two");
      await expectCaretStayedInTheEmbed();
      const active = document.activeElement as HTMLTextAreaElement;
      expect([active.selectionStart, active.selectionEnd, active.selectionDirection]).toEqual([2, 7, "backward"]);
    });
  });

  it("keeps Shift+Tab (outdent) in the embed", async () => {
    await withHostPage(async (root) => {
      const editor = await editInsideEmbed(root);
      // Indent first: `kid-two` starts as a direct child of the embed root, and
      // outdenting a direct child out of the embed root is refused by design.
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(doc.byId["kid-two"]?.parent).toBe("kid-one"));
      const nested = await vi.waitFor(() => {
        const element = root.querySelector<HTMLTextAreaElement>(
          `.embed-block [data-block-id="kid-two"] textarea.block-editor`,
        );
        expect(element).not.toBeNull();
        return element!;
      });
      nested.setSelectionRange(2, 7, "forward");
      nested.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true }),
      );
      await vi.waitFor(() => expect(doc.byId["kid-two"]?.parent).toBe("target"));
      expect(editingId()).toBe("kid-two");
      await expectCaretStayedInTheEmbed();
      const active = document.activeElement as HTMLTextAreaElement;
      expect([active.selectionStart, active.selectionEnd, active.selectionDirection]).toEqual([2, 7, "forward"]);
    });
  });

  it("keeps Alt+Shift+Up (move block up) in the embed", async () => {
    await withHostPage(async (root) => {
      const editor = await editInsideEmbed(root);
      editor.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowUp", altKey: true, shiftKey: true, bubbles: true, cancelable: true }),
      );
      await vi.waitFor(() => expect(doc.byId["target"]?.children).toEqual(["kid-two", "kid-one"]));
      expect(editingId()).toBe("kid-two");
      await expectCaretStayedInTheEmbed();
    });
  });
});

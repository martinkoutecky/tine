import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { editingId } from "../editorController";
import { installKeybindings } from "../keybindings";
import { deleteRenderedTextSelection } from "../editor/renderedSelectionDelete";
import { Block } from "./Block";
import { RefBlocks } from "./RefBlocks";

// Delete/Backspace pressed while a RENDERED (not-editing) block's text is
// selected must delete that text (OG contenteditable parity). Without the
// handling the keypress reached no editor and died silently: selection stayed,
// nothing changed — Martin's "select text and hit delete, nothing happens".
beforeAll(async () => {
  await initParser();
});

const PAGE_NAME = "RenderSel";

function page(roots: string[]): FeedPage {
  return {
    name: PAGE_NAME,
    kind: "page",
    title: PAGE_NAME,
    preBlock: null,
    roots,
    format: "md",
    readOnly: false,
    guide: false,
  };
}

function node(
  id: string,
  raw: string,
  parent: string | null,
  children: string[] = [],
): StoreNode {
  return { id, raw, collapsed: false, parent, page: PAGE_NAME, children };
}

function mount(ids: string[], extra: Record<string, StoreNode> = {}) {
  const byId: Record<string, StoreNode> = { ...extra };
  for (const id of ids) byId[id] = extra[id];
  setDoc({
    byId,
    pages: [page(ids)],
    feed: [PAGE_NAME],
    loaded: true,
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const disposes = ids.map((id) => render(() => <Block id={id} />, host));
  return {
    dispose: () => disposes.forEach((d) => d()),
  };
}

/** The first rendered TEXT node containing `needle` inside a block's wrapper. */
function renderedTextNode(blockId: string, needle: string): Text | null {
  const row = document.querySelector(`[data-block-id="${blockId}"]`);
  const wrapper = row?.querySelector(":scope > .block-main .block-content-wrapper") ?? null;
  if (!wrapper) return null;
  const walker = document.createTreeWalker(wrapper, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if ((n.textContent ?? "").includes(needle)) return n as Text;
  }
  return null;
}

/** Select `start..end` of the FIRST text node that contains `needle` inside
 *  the given block's rendered wrapper. Returns false when not found. */
function selectRenderedText(blockId: string, needle: string, start: number, end: number): boolean {
  const row = document.querySelector(`[data-block-id="${blockId}"]`);
  const wrapper = row?.querySelector(":scope > .block-main .block-content-wrapper") ?? null;
  if (!wrapper) return false;
  const walker = document.createTreeWalker(wrapper, NodeFilter.SHOW_TEXT);
  let t: Text | null = null;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if ((n.textContent ?? "").includes(needle)) {
      t = n as Text;
      break;
    }
  }
  if (!t) return false;
  const range = document.createRange();
  range.setStart(t, start);
  range.setEnd(t, end);
  const sel = window.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(range);
  return true;
}

function keydown(key: string) {
  window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}

afterEach(() => {
  window.getSelection()?.removeAllRanges();
  resetStore();
  document.body.innerHTML = "";
});

describe("rendered-text selection delete (OG parity)", () => {
  it("Delete removes a rendered selection and opens the editor at the join point", () => {
    const m = mount(["a", "b"], {
      a: node("a", "hello world sample text", null),
      b: node("b", "second block stays", null),
    });
    const disposeKeys = installKeybindings();
    try {
      expect(selectRenderedText("a", "hello world sample text", 6, 11)).toBe(true); // "world"
      expect(editingId()).toBeNull();
      keydown("Delete");
      expect(doc.byId["a"].raw).toBe("hello  sample text");
      expect(doc.byId["b"].raw).toBe("second block stays");
      expect(editingId()).toBe("a");
    } finally {
      disposeKeys();
      m.dispose();
    }
  });

  it("Backspace does the same for a selection at the block start", () => {
    const m = mount(["a"], { a: node("a", "hello world sample text", null) });
    const disposeKeys = installKeybindings();
    try {
      expect(selectRenderedText("a", "hello world sample text", 0, 5)).toBe(true); // "hello"
      keydown("Backspace");
      expect(doc.byId["a"].raw).toBe(" world sample text");
      expect(editingId()).toBe("a");
    } finally {
      disposeKeys();
      m.dispose();
    }
  });

  it("preserves hidden property lines (id::) when splicing visible text", () => {
    const m = mount(["a"], { a: node("a", "visible text here\nid:: 12345678-abcd", null) });
    const disposeKeys = installKeybindings();
    try {
      expect(selectRenderedText("a", "visible text here", 8, 12)).toBe(true); // "text"
      keydown("Delete");
      expect(doc.byId["a"].raw).toBe("visible  here\nid:: 12345678-abcd");
    } finally {
      disposeKeys();
      m.dispose();
    }
  });

  it("refuses cross-block selections (no data loss guessing)", () => {
    const m = mount(["a", "b"], {
      a: node("a", "hello world", null),
      b: node("b", "second block", null),
    });
    try {
      // Anchor the range in the rendered TEXT of each block — that is what a
      // real cross-block drag selects. (Addressing `.block-content-wrapper` by
      // child index instead pinned the number of Solid insert markers inside
      // `.block-content`, which is not a contract of this behavior.)
      const textA = renderedTextNode("a", "hello world");
      const textB = renderedTextNode("b", "second block");
      expect(textA && textB).toBeTruthy();
      const range = document.createRange();
      range.setStart(textA!, 0);
      // jsdom requires points in order; a cross-block range is what we're testing
      range.setEnd(textB!, 6);
      const sel = window.getSelection();
      sel?.removeAllRanges();
      sel?.addRange(range);
      expect(deleteRenderedTextSelection()).toBe(false);
      expect(doc.byId["a"].raw).toBe("hello world");
      expect(doc.byId["b"].raw).toBe("second block");
    } finally {
      m.dispose();
    }
  });

  it("refuses on a read-only page", () => {
    const ro: FeedPage = { ...page(["a"]), readOnly: true };
    setDoc({
      byId: { a: node("a", "hello world sample text", null) },
      pages: [ro],
      feed: [PAGE_NAME],
      loaded: true,
    });
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <Block id="a" />, host);
    const disposeKeys = installKeybindings();
    try {
      expect(selectRenderedText("a", "hello world sample text", 0, 5)).toBe(true);
      keydown("Delete");
      expect(doc.byId["a"].raw).toBe("hello world sample text");
      expect(editingId()).toBeNull();
    } finally {
      disposeKeys();
      dispose();
    }
  });

  // Linked/unlinked references, embeds and query results render through
  // RefBlocks, which emits the SAME .ls-block > .block-main >
  // .block-content-wrapper shape and carries the SOURCE block's id. It is a
  // deliberately read-only renderer over a query result, so a rendered
  // selection there must not splice the live block's source — the offsets would
  // come from a different renderer's tree than the raw being cut.
  //
  // NOT causal today: this already no-ops because RefBlocks emits no inline
  // span metadata, so editorOffsetFromRenderedRange returns null. That is an
  // accident of the ref renderer, not a decision — the explicit .ref-block
  // guard states the boundary, and this test pins it so a future span-emitting
  // ref renderer cannot silently turn read-only surfaces into editable ones.
  it("refuses a selection inside a reference/embed/query rendering of a live block", () => {
    setDoc({
      byId: { a: node("a", "hello world sample text", null) },
      pages: [page(["a"])],
      feed: [PAGE_NAME],
      loaded: true,
    });
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(
      () => (
        <RefBlocks
          blocks={[{ id: "a", raw: "hello world sample text", collapsed: false, children: [] }]}
          page={PAGE_NAME}
          pageKind="page"
        />
      ),
      host,
    );
    const disposeKeys = installKeybindings();
    try {
      expect(document.querySelector('[data-block-id="a"]')?.classList.contains("ref-block")).toBe(true);
      expect(selectRenderedText("a", "hello world sample text", 6, 11)).toBe(true);
      expect(deleteRenderedTextSelection()).toBe(false);
      keydown("Delete");
      expect(doc.byId["a"].raw).toBe("hello world sample text");
      expect(editingId()).toBeNull();
    } finally {
      disposeKeys();
      dispose();
    }
  });

  it("ignores a collapsed (caret-only) selection and absent selection", () => {
    const m = mount(["a"], { a: node("a", "hello world", null) });
    try {
      expect(deleteRenderedTextSelection()).toBe(false);
      expect(doc.byId["a"].raw).toBe("hello world");
    } finally {
      m.dispose();
    }
  });
});

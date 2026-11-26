import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { doc, setDoc, type Node as StoreNode } from "../document/model";
import { editingId } from "../editorController";
import { installKeybindings } from "../keybindings";
import { Block } from "./Block";

// Some rendered constructs have no exact mapping from their rendered text to
// source: inline HTML, page references, tags, math, entities. A Delete or
// Backspace over a rendered selection whose end lies strictly inside one used
// to snap that end to the nearer edge of the whole construct, so the key
// deleted text that was not selected (`a <b>hello world</b> c`, select `lo w`,
// Delete: saved `a  c`). Such a selection now writes nothing.
beforeAll(async () => {
  await initParser();
});

const PAGE_NAME = "CoarseSel";

function mount(raw: string) {
  const node: StoreNode = { id: "a", raw, collapsed: false, parent: null, page: PAGE_NAME, children: [] };
  setDoc({
    byId: { a: node },
    pages: [{ name: PAGE_NAME, kind: "page", title: PAGE_NAME, preBlock: null, roots: ["a"], format: "md", readOnly: false, guide: false }],
    feed: [PAGE_NAME],
    loaded: true,
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  return render(() => <Block id="a" />, host);
}

/** The `index`-th rendered text node whose text is exactly `text`. */
function textNode(text: string, index = 0): Text {
  const wrapper = document.querySelector('[data-block-id="a"] > .block-main .block-content-wrapper')!;
  const walker = document.createTreeWalker(wrapper, NodeFilter.SHOW_TEXT);
  let seen = 0;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.textContent === text && seen++ === index) return n as Text;
  }
  throw new Error(`no rendered text node ${JSON.stringify(text)}`);
}

function select(start: [Text, number], end: [Text, number]): Selection {
  const range = document.createRange();
  range.setStart(start[0], start[1]);
  range.setEnd(end[0], end[1]);
  const sel = window.getSelection()!;
  sel.removeAllRanges();
  sel.addRange(range);
  return sel;
}

function press(key: "Delete" | "Backspace") {
  window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}

/** Select, press `key`, and return the block's saved text afterwards. */
function deleteSelection(raw: string, pick: () => [[Text, number], [Text, number]], key: "Delete" | "Backspace" = "Delete") {
  const dispose = mount(raw);
  const disposeKeys = installKeybindings();
  try {
    const [start, end] = pick();
    const sel = select(start, end);
    const selected = sel.toString();
    press(key);
    return { raw: doc.byId["a"].raw, editing: editingId(), selected, kept: sel.rangeCount === 1 ? sel.toString() : "" };
  } finally {
    disposeKeys();
    dispose();
  }
}

afterEach(() => {
  window.getSelection()?.removeAllRanges();
  resetStore();
  document.body.innerHTML = "";
});

describe("rendered-selection delete with an end inside a construct that has no exact text mapping", () => {
  it("both ends inside inline HTML: nothing is written and the selection stays", () => {
    const raw = "a <b>hello world</b> c";
    const r = deleteSelection(raw, () => {
      const t = textNode("hello world");
      return [[t, 3], [t, 7]];
    });
    expect(r.selected).toBe("lo w");
    expect(r.raw).toBe(raw);
    expect(r.editing).toBeNull();
    expect(r.kept).toBe("lo w");
  });

  it("Backspace refuses the same selection", () => {
    const raw = "a <b>hello world</b> c";
    const r = deleteSelection(raw, () => {
      const t = textNode("hello world");
      return [[t, 3], [t, 7]];
    }, "Backspace");
    expect(r.raw).toBe(raw);
    expect(r.editing).toBeNull();
  });

  it("start in prose, end inside inline HTML: nothing is written", () => {
    const raw = "a <b>hello world</b> c";
    const r = deleteSelection(raw, () => [[textNode("a "), 1], [textNode("hello world"), 3]]);
    expect(r.selected).toBe(" hel");
    expect(r.raw).toBe(raw);
    expect(r.editing).toBeNull();
  });

  it("start inside inline HTML, end in prose: nothing is written", () => {
    const raw = "a <b>hello world</b> c";
    const r = deleteSelection(raw, () => [[textNode("hello world"), 3], [textNode(" c"), 1]]);
    expect(r.selected).toBe("lo world ");
    expect(r.raw).toBe(raw);
    expect(r.editing).toBeNull();
  });

  it("ends inside two different constructs (inline HTML, then a page reference): nothing is written", () => {
    const raw = "x <span>hello world</span> [[two words]] y";
    const r = deleteSelection(raw, () => [[textNode("hello world"), 3], [textNode("two words"), 2]]);
    expect(r.raw).toBe(raw);
    expect(r.editing).toBeNull();
  });

  it("ends inside two inline HTML elements: nothing is written", () => {
    const raw = "a <b>hello</b> mid <i>there</i> c";
    const r = deleteSelection(raw, () => [[textNode("hello"), 2], [textNode("there"), 2]]);
    expect(r.selected).toBe("llo mid th");
    expect(r.raw).toBe(raw);
    expect(r.editing).toBeNull();
  });

  // Every inline construct the renderer marks as coarse is held by the same
  // check; these are the ones a selection can end inside.
  it.each([
    ["a page reference", "see [[hello world]] now", "hello world", 3, 7],
    ["a tag", "x #longtag y", "longtag", 1, 4],
    ["math", "x $a+b+c$ y", "a+b+c", 1, 3],
  ])("both ends inside %s: nothing is written", (_kind, raw, text, from, to) => {
    const r = deleteSelection(raw, () => {
      const t = textNode(text);
      return [[t, from], [t, to]];
    });
    expect(r.raw).toBe(raw);
    expect(r.editing).toBeNull();
  });
});

describe("rendered-selection delete: unchanged cases", () => {
  it("a selection covering a whole inline HTML element deletes the element", () => {
    const r = deleteSelection("a <b>hello world</b> c", () => {
      const t = textNode("hello world");
      return [[t, 0], [t, 11]];
    });
    expect(r.raw).toBe("a  c");
    expect(r.editing).toBe("a");
  });

  it("a selection from prose across a whole element into prose deletes all of it", () => {
    const r = deleteSelection("a <b>hello world</b> c", () => [[textNode("a "), 1], [textNode(" c"), 1]]);
    expect(r.selected).toBe(" hello world ");
    expect(r.raw).toBe("ac");
  });

  it("a selection in plain prose deletes exactly the selected text", () => {
    const r = deleteSelection("a <b>hello world</b> some prose", () => {
      const t = textNode(" some prose");
      return [[t, 1], [t, 6]];
    });
    expect(r.raw).toBe("a <b>hello world</b> prose");
  });

  it("a selection inside ==highlight== text deletes exactly the selected text", () => {
    const r = deleteSelection("say ==hello world== now", () => {
      const t = textNode("hello world");
      return [[t, 3], [t, 7]];
    });
    expect(r.raw).toBe("say ==helorld== now");
    expect(r.editing).toBe("a");
  });

  it("a selection inside a markdown link's label deletes exactly the selected text", () => {
    const r = deleteSelection("x [hello world](https://e.com) y", () => {
      const t = textNode("hello world");
      return [[t, 3], [t, 7]];
    });
    expect(r.raw).toBe("x [helorld](https://e.com) y");
  });
});

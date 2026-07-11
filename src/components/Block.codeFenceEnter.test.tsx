import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import { startEditing } from "../editorController";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function blk(id: string, raw: string): BlockDto {
  return { id, raw, collapsed: false, children: [] };
}

function page(name: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "page", title: name, pre_block: null, blocks };
}

function pressEnter(ta: HTMLTextAreaElement, caret: number) {
  ta.focus();
  ta.selectionStart = caret;
  ta.selectionEnd = caret;
  ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
}

// GH #66: Enter INSIDE a fenced code block must insert a newline and stay in the
// same block, not split off a new bullet (which breaks the fence).
describe("Enter inside a code fence", () => {
  it("inserts a newline and does NOT split the block", () => {
    loadSingle(page("Code", [blk("code-1", "```js\nconst x = 1\n```")]));
    const id = pageByName("Code")!.roots[0];

    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));

    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement | null;
      expect(ta).not.toBeNull();
      // Caret at the end of the `const x = 1` content line (inside the fence).
      const caret = ta!.value.indexOf("const x = 1") + "const x = 1".length;
      pressEnter(ta!, caret);

      // The page still has exactly one root block — Enter did NOT split.
      expect(pageByName("Code")!.roots.length).toBe(1);
      // A newline was inserted inside the fence content.
      const raw = doc.byId[id]?.raw ?? ta!.value;
      expect(raw).toContain("const x = 1\n\n```");
    } finally {
      dispose();
    }
  });

  it("stays inside a four-backtick fence after a shorter three-backtick run", () => {
    loadSingle(page("Code", [blk("code-4", "````js\n```\nconst x = 1\n````") ]));
    const id = pageByName("Code")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement;
      const caret = ta.value.indexOf("const x = 1") + "const x = 1".length;
      pressEnter(ta, caret);
      expect(pageByName("Code")!.roots).toEqual([id]);
      expect(doc.byId[id].raw).toContain("const x = 1\n\n````");
    } finally {
      dispose();
    }
  });

  it("continues a freshly typed opening fence in the same block", () => {
    loadSingle(page("Code", [blk("opening", "```js") ]));
    const id = pageByName("Code")!.roots[0];
    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));
    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement;
      pressEnter(ta, ta.value.length);
      expect(pageByName("Code")!.roots).toEqual([id]);
      expect(doc.byId[id].raw).toBe("```js\n");
    } finally {
      dispose();
    }
  });

  it("still splits into a new block when Enter is pressed OUTSIDE any fence", () => {
    // Necessity guard: the fence special-case must not swallow ordinary Enter.
    loadSingle(page("Plain", [blk("plain-1", "hello world")]));
    const id = pageByName("Plain")!.roots[0];

    startEditing(id, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Plain")?.roots ?? []}>{(bid) => <Block id={bid} />}</For>
    ));

    try {
      const ta = root.querySelector("textarea") as HTMLTextAreaElement | null;
      expect(ta).not.toBeNull();
      pressEnter(ta!, ta!.value.length);
      // Ordinary text splits into a second block.
      expect(pageByName("Plain")!.roots.length).toBe(2);
    } finally {
      dispose();
    }
  });
});

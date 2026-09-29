import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { endEdit } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

// GH #489: clicking a rendered code card must put the caret where it was
// clicked, in the body-only editor. The card is highlight.js markup with no
// lsdoc span data, so the general click mapper declines; the code-card mapper
// answers from rendered text position. jsdom has no layout, so
// `caretRangeFromPoint` is stubbed to the text position a real engine reports.

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
  delete (document as { caretRangeFromPoint?: unknown }).caretRangeFromPoint;
});

const BODY = "const a = 1;\nconst b = 2;\nconst c = 3;";

function clickCode(
  raw: string,
  landOn: (code: Element) => { node: Node; offset: number },
  init: { name: string },
): { root: HTMLElement; dispose: () => void } {
  const block: BlockDto = { id: `card-${init.name}`, raw, collapsed: false, children: [] };
  const page: PageDto = { name: init.name, kind: "page", title: init.name, pre_block: null, blocks: [block] };
  loadSingle(page);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => (
    <For each={pageByName(init.name)?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  const content = root.querySelector(".block-content") as HTMLElement;
  const code = root.querySelector("pre.code-block > code")!;
  const target = landOn(code);
  (document as unknown as { caretRangeFromPoint: () => Range }).caretRangeFromPoint = () => {
    const range = document.createRange();
    range.setStart(target.node, target.offset);
    range.collapse(true);
    return range;
  };
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  return { root, dispose };
}

function textNodeContaining(code: Element, needle: string): Text {
  const walker = document.createTreeWalker(code, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if ((n.textContent ?? "").includes(needle)) return n as Text;
  }
  throw new Error(`no text node contains ${needle}`);
}

describe("clicking a code block places the caret where clicked (GH #489)", () => {
  it("opens the body-only editor with the caret at the clicked column of the clicked line", () => {
    const { root, dispose } = clickCode(
      `\`\`\`js\n${BODY}\n\`\`\``,
      (code) => {
        const node = textNodeContaining(code, "const b");
        return { node, offset: (node.textContent ?? "").indexOf("const b") + 5 };
      },
      { name: "Card click" },
    );
    try {
      const ta = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      expect(ta.value).toBe(BODY);
      // Not the end of the block (the pre-fix answer): line two, column five.
      expect(ta.selectionStart).toBe(BODY.indexOf("const b") + 5);
      expect(ta.selectionEnd).toBe(ta.selectionStart);
    } finally {
      dispose();
    }
  });

  it("a click at the end of a short line means that line's end, not the block's", () => {
    const { root, dispose } = clickCode(
      `\`\`\`js\n${BODY}\n\`\`\``,
      (code) => {
        const node = textNodeContaining(code, "const a");
        return { node, offset: (node.textContent ?? "").indexOf("const a") + "const a = 1;".length };
      },
      { name: "Card click end" },
    );
    try {
      const ta = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      expect(ta.selectionStart).toBe("const a = 1;".length);
    } finally {
      dispose();
    }
  });
});

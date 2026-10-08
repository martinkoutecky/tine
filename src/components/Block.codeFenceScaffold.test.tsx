import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { startEditing, endEdit } from "../editorController";
import { setAutoPairing } from "../ui";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

// GH #413: typing three backticks to open a code block is an EXPLICIT scaffold
// decision — it yields exactly three opening backticks, inserts the matching
// closing fence, and lands the caret inside (between the fences). It wins over
// the generic symmetric-backtick auto-pairing (which used to leave FOUR
// backticks and no closer). Single inline backticks and all bracket/ref
// pairing are unchanged.
//
// GH #507: the scaffold used to drop the caret straight into the body-only code
// view, which hides the opener line, so a language could no longer be typed.
// The third backtick now opens the same language picker as /Code block, on the
// still-visible opener; choosing a language or pressing Escape then lands the
// caret inside.

beforeAll(() => initParser());

afterEach(() => {
  setAutoPairing(true);
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

let seq = 0;
function mountEditor(raw: string) {
  const block: BlockDto = { id: `sf-${++seq}`, raw, collapsed: false, children: [] };
  const name = `SF${seq}`;
  const page: PageDto = { name, kind: "page", title: name, pre_block: null, blocks: [block] };
  loadSingle(page);
  startEditing(block.id, raw.length);
  const mounted = mount(() => (
    <For each={pageByName(name)?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ));
  return {
    ...mounted,
    blockId: block.id,
    textarea: mounted.root.querySelector("textarea.block-editor") as HTMLTextAreaElement,
  };
}

/** Simulate one keystroke: the browser inserts the char, then onInput runs. */
function typeChar(ta: HTMLTextAreaElement, ch: string) {
  const caret = ta.selectionStart;
  ta.value = ta.value.slice(0, caret) + ch + ta.value.slice(caret);
  ta.setSelectionRange(caret + 1, caret + 1);
  ta.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: ch }));
}

function key(ta: HTMLTextAreaElement, k: string) {
  ta.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));
}

const pickerItems = () => [...document.body.querySelectorAll(".autocomplete .ac-item")];
const pickerLabels = () => pickerItems().map((item) => item.querySelector(".ac-label")?.textContent);

describe("triple-backtick code fence scaffold (GH #413)", () => {
  it("three backticks insert a complete fence scaffold — exactly three, never four — and Escape lands the caret inside", async () => {
    const { textarea, blockId, dispose } = mountEditor("");
    try {
      typeChar(textarea, "`");
      // A single backtick still auto-pairs (unchanged generic behavior).
      expect(textarea.value).toBe("``");
      typeChar(textarea, "`");
      typeChar(textarea, "`");
      // The scaffold replaces the third-backtick pairing: no fourth backtick,
      // and a matching closing fence appears.
      expect(doc.byId[blockId].raw).toBe("```\n\n```");
      expect(doc.byId[blockId].raw).not.toContain("````");
      // GH #507: the language picker is offered first. Escape declines it and
      // swaps to the body-only view with the caret between the fences.
      await vi.waitFor(() => expect(pickerItems().length).toBeGreaterThan(0));
      key(textarea, "Escape");
      await vi.waitFor(() => {
        expect(textarea.value).not.toContain("```");
        expect(textarea.selectionStart).toBe(0);
        expect(textarea.selectionEnd).toBe(0);
      });
      expect(doc.byId[blockId].raw).toBe("```\n\n```");
    } finally {
      dispose();
    }
  });

  it("the scaffold is a whole-block decision: three backticks mid-text keep ordinary inline behavior", () => {
    const { textarea, blockId, dispose } = mountEditor("note ");
    try {
      typeChar(textarea, "`");
      typeChar(textarea, "`");
      typeChar(textarea, "`");
      // No fence scaffold was synthesized inline: the block stays one line of prose.
      expect(doc.byId[blockId].raw).not.toContain("\n");
      expect(doc.byId[blockId].raw.startsWith("note ")).toBe(true);
      expect(pickerItems()).toHaveLength(0);
    } finally {
      dispose();
    }
  });

  it("keeps pairing off behavior: with auto-pairing disabled, three backticks still scaffold", async () => {
    setAutoPairing(false);
    const { textarea, blockId, dispose } = mountEditor("");
    try {
      typeChar(textarea, "`");
      expect(textarea.value).toBe("`"); // no pairing with the toggle off
      typeChar(textarea, "`");
      typeChar(textarea, "`");
      expect(doc.byId[blockId].raw).toBe("```\n\n```");
      await vi.waitFor(() => expect(pickerItems().length).toBeGreaterThan(0));
      key(textarea, "Escape");
      await vi.waitFor(() => {
        expect(textarea.value).not.toContain("```");
        expect(textarea.selectionStart).toBe(0);
      });
    } finally {
      dispose();
    }
  });
});

describe("typed code fence language (GH #507)", () => {
  it("offers the language picker on the still-visible opener line", async () => {
    const { textarea, dispose } = mountEditor("");
    try {
      for (const ch of "```") typeChar(textarea, ch);
      await vi.waitFor(() => expect(pickerItems().length).toBeGreaterThan(0));
      expect(textarea.value).toBe("```\n\n```");
      expect(textarea.selectionStart).toBe(3);
    } finally {
      dispose();
    }
  });

  it("lets the user type a language after the fence, then lands in the code body", async () => {
    const { textarea, blockId, dispose } = mountEditor("");
    try {
      for (const ch of "```") typeChar(textarea, ch);
      await vi.waitFor(() => expect(pickerItems().length).toBeGreaterThan(0));
      for (const ch of "pyth") typeChar(textarea, ch);
      expect(doc.byId[blockId].raw).toBe("```pyth\n\n```");
      await vi.waitFor(() => expect(pickerLabels()).toContain("Python"));

      const python = pickerItems().find((item) => item.querySelector(".ac-label")?.textContent === "Python")!;
      python.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(doc.byId[blockId].raw).toBe("```python\n\n```"));
      expect(textarea.value).not.toContain("```");
      expect(textarea.selectionStart).toBe(0);
    } finally {
      dispose();
    }
  });
});

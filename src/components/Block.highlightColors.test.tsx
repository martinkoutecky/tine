import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto } from "../types";
import { Block } from "./Block";
import { editorIn } from "../tests/editorDriver";

// the selection toolbar's second row of highlight colours (UI-HLC-INLINE-COLOURS).
// This file mounts the block editor, so it runs for the textarea editor (render suite) and the
// CodeMirror editor (vitest.cm6.config.ts).
const platform = vi.hoisted(() => ({ mobile: false }));
vi.mock("../inputCapability", async (original) => ({
  ...(await original<typeof import("../inputCapability")>()),
  touchPrimary: () => platform.mobile,
  touchCapable: () => platform.mobile,
  touchGestures: () => false,
}));

const RED = '<mark style="background-color: var(--ls-highlight-color-red, #fdd8d8)">';
const BLUE = '<mark style="background-color: var(--ls-highlight-color-blue, #cee7fe)">';

beforeAll(() => initParser());
afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
  platform.mobile = false;
});

function mountEditor(raw: string, format: "md" | "org" = "md", caret = 0) {
  const block: BlockDto = { id: "highlight-colors", raw, collapsed: false, children: [] };
  loadSingle({ name: "Colours", kind: "page", title: "Colours", pre_block: null, blocks: [block], format });
  startEditing(block.id, caret);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render((): JSX.Element => (
    <For each={pageByName("Colours")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  return { root, dispose, editor: editorIn(root) };
}

const swatch = (root: HTMLElement, color: string) =>
  root.querySelector<HTMLButtonElement>(`.sel-toolbar .sel-toolbar-colors [data-highlight-color="${color}"]`);

describe("highlight colour row under the selection toolbar", () => {
  it("writes the colour, replaces it with a second one, removes it, each one Undo step", async () => {
    const { editor, root, dispose } = mountEditor("say hello world");
    try {
      editor.selectByUser(4, 9);
      await vi.waitFor(() => expect(swatch(root, "red")).not.toBeNull());
      expect([...root.querySelectorAll(".sel-toolbar-colors [data-highlight-color]")].map((b) => b.getAttribute("data-highlight-color")))
        .toEqual(["yellow", "red", "pink", "green", "blue", "purple", "gray", ""]);

      swatch(root, "red")!.click();
      await vi.waitFor(() => expect(doc.byId["highlight-colors"].raw).toBe(`say ${RED}hello</mark> world`));
      await vi.waitFor(() => expect(editor.value().slice(...editor.selection())).toBe("hello"));

      swatch(root, "blue")!.click();
      await vi.waitFor(() => expect(doc.byId["highlight-colors"].raw).toBe(`say ${BLUE}hello</mark> world`));
      await vi.waitFor(() => expect(editor.value().slice(...editor.selection())).toBe("hello"));

      swatch(root, "")!.click();
      await vi.waitFor(() => expect(doc.byId["highlight-colors"].raw).toBe("say hello world"));
      await vi.waitFor(() => expect(editor.selection()).toEqual([4, 9]));

      undo();
      expect(doc.byId["highlight-colors"].raw).toBe(`say ${BLUE}hello</mark> world`);
      undo();
      expect(doc.byId["highlight-colors"].raw).toBe(`say ${RED}hello</mark> world`);
      undo();
      expect(doc.byId["highlight-colors"].raw).toBe("say hello world");
    } finally {
      dispose();
    }
  });

  it("leaves a page link as it is", async () => {
    const { editor, root, dispose } = mountEditor("see [[Some page]] now");
    try {
      editor.selectByUser(4, 17);
      await vi.waitFor(() => expect(swatch(root, "red")).not.toBeNull());
      swatch(root, "red")!.click();
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(doc.byId["highlight-colors"].raw).toBe("see [[Some page]] now");
    } finally {
      dispose();
    }
  });

  it("shows no colour row in an Org block", async () => {
    const { editor, root, dispose } = mountEditor("say hello world", "org");
    try {
      editor.selectByUser(4, 9);
      await vi.waitFor(() => expect(root.querySelector(".sel-toolbar")).not.toBeNull());
      expect(root.querySelector(".sel-toolbar-colors")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("opens the row from H on a phone, which keeps the plain highlight in it", async () => {
    platform.mobile = true;
    const { editor, root, dispose } = mountEditor("say hello world");
    try {
      editor.selectByUser(4, 9);
      await vi.waitFor(() => expect(root.querySelector(".sel-toolbar")).not.toBeNull());
      expect(root.querySelector(".sel-toolbar-colors")).toBeNull();
      root.querySelector<HTMLButtonElement>('.sel-toolbar-secondary [data-selection-action="highlight"]')!.click();
      await vi.waitFor(() => expect(swatch(root, "green")).not.toBeNull());
      expect(doc.byId["highlight-colors"].raw).toBe("say hello world");
      root.querySelector<HTMLButtonElement>(".sel-toolbar-colors [data-highlight-plain]")!.click();
      await vi.waitFor(() => expect(doc.byId["highlight-colors"].raw).toBe("say ==hello== world"));
    } finally {
      dispose();
    }
  });

  // Merge review of b218d18e3, finding 1: a literal buffer (code body, calculator) gets no colours.
  it.each([
    ["a fenced code body", "```js\nconst answer = 42\n```", "answer"],
    ["a calculator", "```calc\n2 + 2\n```", "+ 2"],
  ])("offers no colours in %s", async (_label, raw, word) => {
    // The caret on the body's first line opens the body-only (literal) view.
    const { editor, root, dispose } = mountEditor(raw, "md", raw.indexOf("\n") + 1);
    try {
      await vi.waitFor(() => expect(editor.value()).not.toBe(raw));
      const from = editor.value().indexOf(word);
      editor.selectByUser(from, from + word.length);
      await vi.waitFor(() => expect(root.querySelector(".sel-toolbar")).not.toBeNull());
      expect(root.querySelector(".sel-toolbar-colors")).toBeNull();
      expect(doc.byId["highlight-colors"].raw).toBe(raw);
    } finally {
      dispose();
    }
  });

  // Finding 5: a press that changes nothing leaves the typing burst whole.
  it.each([["no colour on uncoloured text", ""], ["a refused colour", "refused"], ["the colour already there", "same"]])(
    "keeps typing one Undo step across %s",
    async (_label, kind) => {
      const start = kind === "refused" ? "see [[P]]" : kind === "same" ? `say ${RED}hello</mark>` : "say";
      const { editor, root, dispose } = mountEditor(start);
      try {
        editor.select(start.length);
        editor.type(" hi");
        await vi.waitFor(() => expect(doc.byId["highlight-colors"].raw).toBe(`${start} hi`));
        const target = kind === "refused" ? "[[P]]" : kind === "same" ? "hello" : "hi";
        const from = editor.value().indexOf(target);
        editor.selectByUser(from, from + target.length);
        await vi.waitFor(() => expect(swatch(root, "red")).not.toBeNull());
        swatch(root, kind === "" ? "" : "red")!.click();
        await new Promise((resolve) => setTimeout(resolve, 20));
        expect(doc.byId["highlight-colors"].raw).toBe(`${start} hi`);
        editor.select(editor.value().length);
        editor.type(" there");
        await vi.waitFor(() => expect(doc.byId["highlight-colors"].raw).toBe(`${start} hi there`));
        undo();
        expect(doc.byId["highlight-colors"].raw).toBe(start);
      } finally {
        dispose();
      }
    },
  );
});

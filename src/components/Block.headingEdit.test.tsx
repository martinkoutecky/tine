import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { endEdit, startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { editorIn } from "../tests/editorDriver";
import { Block } from "./Block";

// CM6-HEADING. A heading block is sized by the view (`heading-text hN` on its header line); entering edit must
// not change that size, or the page jumps. OG's editor textarea takes `hN` from the block's properties
// (`components/editor.cljs get-editor-style-class` reads `(:heading (:block/properties block))`, and
// `.editor-inner .hN.uniline-block` sizes it), not from the editor text, where `heading::` is hidden.
// Runs with the textarea editor in the default render config and with CM6 in the CM6 census.

beforeAll(() => initParser());
afterEach(() => { endEdit("page-navigation"); resetStore(); document.body.innerHTML = ""; });

function edit(raw: string, properties?: [string, string][], heading_level?: number) {
  loadSingle({ name: "HeadingEdit", title: "HeadingEdit", kind: "page", pre_block: null, format: "md",
    blocks: [{ id: "h", raw, collapsed: false, properties, heading_level, children: [] }] });
  const root = document.createElement("div"); document.body.append(root);
  render(() => <For each={pageByName("HeadingEdit")?.roots ?? []}>{id => <Block id={id} />}</For>, root);
  startEditing("h", 0);
  return { root, editor: editorIn(root) };
}

/** The heading size the open editor shows its first line in, or null. */
function shownLevel(root: HTMLElement, editor: ReturnType<typeof editorIn>): number | null {
  const sized = editor.kind === "textarea" ? editor.element : root.querySelector(".cm-line .heading-text");
  const level = sized?.className.match(/\bh([1-6])\b/);
  return level ? Number(level[1]) : null;
}

describe("a heading block edits at the view's heading size", () => {
  it("takes the level from the heading:: property that the editor text no longer contains", () => {
    for (const level of [1, 2, 3, 4, 5, 6]) {
      const { root, editor } = edit(`Title\nheading:: ${level}`, [["heading", String(level)]]);
      expect(editor.value(), "the property is hidden from the editor text").toBe("Title");
      expect(shownLevel(root, editor), `heading:: ${level}`).toBe(level);
      endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
    }
  });
  it("derives heading:: true from the depth, as the view does", () => {
    const { root, editor } = edit("Title\nheading:: true", [["heading", "true"]]);
    expect(shownLevel(root, editor)).toBe(1);
  });
  it("keeps the size of a markdown # heading", () => {
    const { root, editor } = edit("## Title", undefined, 2); // the backend ships the ATX level with the block
    expect(shownLevel(root, editor)).toBe(2);
  });
  it("leaves a block without a heading at body size", () => {
    const { root, editor } = edit("Title\nfoo:: bar", [["foo", "bar"]]);
    expect(shownLevel(root, editor)).toBeNull();
  });
  it("sizes only a single-line heading in the textarea (OG uniline-block); CM6 sizes the header line either way", () => {
    const { root, editor } = edit("Title\nsecond line\nheading:: 2", [["heading", "2"]]);
    expect(shownLevel(root, editor)).toBe(editor.kind === "textarea" ? null : 2);
  });
});

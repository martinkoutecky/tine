import { expect, it } from "vitest";
import { Block } from "./Block";
import { blockProperty, node, pageByName } from "../document";
import { loadSingle } from "../document/workingSet";
import { clearSeededFacets } from "../render/facets";
import { endEdit, startEditing } from "../editorController";
import { editorIn, type EditorDriver } from "../tests/editorDriver";
import { blk, installBlockEditorLifecycle, mount, page } from "../tests/blockEditorTestkit";

installBlockEditorLifecycle();

function typePrefix(editor: EditorDriver, text: string) {
  editor.focus();
  for (const ch of text) {
    const value = editor.value() + ch;
    editor.replaceText(value);
    editor.select(value.length, value.length);
    editor.reportInput({ bubbles: true, inputType: "insertText", data: ch });
  }
}

const bodies = [
  { name: "Markdown code", format: "md", open: "```js\n", close: "\n```" },
  { name: "tilde code", format: "md", open: "~~~text\n", close: "\n~~~" },
  { name: "Org source", format: "org", open: "#+BEGIN_SRC js\n", close: "\n#+END_SRC" },
  { name: "Org example", format: "org", open: "#+BEGIN_EXAMPLE\n", close: "\n#+END_EXAMPLE" },
  { name: "calculator", format: "md", open: "```calc\n", close: "\n```" },
  { name: "mixed Markdown with metadata", format: "md",
    open: "intro\nid:: aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n```js\n",
    visibleOpen: "intro\n```js\n", close: "\n```\noutro  \n~~~py\nother\n~~~" },
  { name: "mixed Org", format: "org", open: "intro\n#+BEGIN_SRC js\n", close: "\n#+END_SRC\noutro" },
] as const;

// Include neighboring bullet, heading and quote prefixes.
// Each starts from an empty body, where an exact visible-buffer trigger can fire.
const cases = bodies.flatMap(body => ["1. ", "- ", "# ", "> "].map(prefix => ({ ...body, prefix })));
it.each(cases)("keeps $prefix literal in a projected $name body", body => {
  const { prefix } = body;
  const raw = body.open + body.close;
  loadSingle({ ...page("Literal prefixes", [blk("literal", raw), blk("after", "untouched")]), format: body.format });
  clearSeededFacets();
  startEditing("literal", "visibleOpen" in body ? body.visibleOpen.length : body.open.length);
  const { root, dispose } = mount(() => <Block id="literal" />);
  try {
    const editor = editorIn(root);
    expect(editor.value()).toBe("");
    typePrefix(editor, prefix);
    expect(editor.value(), prefix).toBe(prefix);
    expect(node("literal")!.raw, prefix).toBe(body.open + prefix + body.close);
    expect(blockProperty("literal", "logseq.order-list-type"), prefix).toBeNull();
    expect(pageByName("Literal prefixes")!.roots).toEqual(["literal", "after"]);
    expect(node("after")!.raw).toBe("untouched");
    // Leaving the editor must use the same reversible body commit.
    editor.reportBlur();
    expect(node("literal")!.raw, prefix).toBe(body.open + prefix + body.close);
  } finally { dispose(); endEdit("page-navigation"); }
});

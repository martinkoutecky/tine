import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { editingId, endEdit, startEditing } from "../editorController";
import { node, resetStore, selectedIds } from "../document";
import { loadSingle } from "../document/workingSet";
import { editorIn } from "../tests/editorDriver";
import { initParser } from "../render/parse";

let dispose: (() => void) | undefined;
beforeAll(async () => {
  await initParser();
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
});
afterEach(() => {
  dispose?.(); dispose = undefined;
  endEdit("page-navigation"); resetStore(); document.body.innerHTML = "";
});

for (const surface of ["main", "pane:second", "sidebar", "ref:example", "embed:example"]) {
  it.each([
    ["md", "```js\nx\n```"],
    ["org", "#+BEGIN_SRC js\nx\n#+END_SRC"],
    ["md", "intro\n```js\nx\n```\nafter"],
    ["org", "intro\n#+BEGIN_SRC js\nx\n#+END_SRC\nafter"],
  ] as const)(`reveals editable %s source until the second Escape on ${surface} (GH #865)`, (format, raw) => {
    loadSingle({ name: "Code", title: "Code", kind: "page", pre_block: null, format,
      blocks: [{ id: "code", raw, collapsed: false, children: [] }] });
    startEditing("code", raw.indexOf("\nx\n") + 1, "code-test", surface);
    const root = document.createElement("div"); document.body.append(root);
    dispose = render(() => <Cm6BlockEditor id="code" surface={surface} scope={null} />, root);
    const editor = editorIn(root);
    expect(editor.value()).toBe("x");
    editor.select(1); editor.type("y");
    const expected = raw.replace("\nx\n", "\nxy\n");
    editor.keydown({ key: "Escape" });
    expect(editingId()).toBe("code");
    expect(editor.value()).toBe(expected);
    expect(editor.selection()).toEqual([expected.length, expected.length]);
    expect(node("code").raw).toBe(expected);
    // Selecting inside the body must not immediately project the fences away.
    editor.selectByUser(expected.indexOf("\nxy\n") + 2);
    expect(editor.value()).toBe(expected);
    editor.select(0); editor.type("prefix\n");
    expect(editor.value()).toBe("prefix\n" + expected);
    expect(node("code").raw).toBe("prefix\n" + expected);
    editor.keydown({ key: "Escape" });
    expect(editingId()).toBeNull();
    expect(selectedIds()).toEqual(["code"]);
  });
}

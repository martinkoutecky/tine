import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { editorIn } from "../tests/editorDriver";
import { loadSingle } from "../document/workingSet";
import { resetStore } from "../document";
import { startEditing, endEdit } from "../editorController";
import { initParser } from "../render/parse";
import { Block } from "./Block";

beforeAll(() => initParser());
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

it("keeps the caret after :: when the input inserted both characters together (census D8)", () => {
  loadSingle({ name: "Completion input", title: "Completion input", kind: "page", pre_block: null,
    blocks: [{ id: "completion-input", raw: "", collapsed: false, children: [] }] });
  startEditing("completion-input", 0);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Block id="completion-input" />, root);
  try {
    const editor = editorIn(root);
    editor.type("::");
    expect(editor.value()).toBe("::");
    expect(editor.selection()).toEqual([2, 2]);
  } finally { dispose(); }
});

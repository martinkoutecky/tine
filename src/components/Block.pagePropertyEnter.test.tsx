import { editorIn, type EditorDriver } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc, setDoc } from "../document/model";
import { editingId, startEditing } from "../editorController";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => initParser());
afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

const block = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const page = (blocks: BlockDto[]): PageDto => ({ name: "Properties", kind: "page", title: "Properties", pre_block: null, blocks });
const mount = (node: () => JSX.Element) => {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(node, root) };
};
const pressEnter = (textarea: EditorDriver) => {
  textarea.focus();
  textarea.select(textarea.value().length, textarea.value().length);
  textarea.keydown({ key: "Enter", bubbles: true, cancelable: true });
};
const replaceValue = (textarea: EditorDriver, value: string) => {
  textarea.input(value, value.length, { inputType: "insertText", data: value });
};

describe("first-block page-property entry (GH #138)", () => {
  it.each([false, true])("keeps plain Enter in the property block and exits on the following empty line (marked header: %s)", (marked) => {
    loadSingle(page([block("props", "alias:: book"), block("body", "Reading list")]));
    if (marked) setDoc("byId", "props", "originatedFromPageHeader", true);
    startEditing("props", "alias:: book".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Properties")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = editorIn(root);
      pressEnter(textarea);
      expect(pageByName("Properties")!.roots).toEqual(["props", "body"]);
      expect(textarea.value()).toBe("alias:: book\n");

      replaceValue(textarea, "alias:: book\ntags:: blah");
      pressEnter(textarea);
      expect(pageByName("Properties")!.roots).toEqual(["props", "body"]);
      expect(textarea.value()).toBe("alias:: book\ntags:: blah\n");

      pressEnter(textarea);
      const roots = pageByName("Properties")!.roots;
      expect(roots).toHaveLength(3);
      expect(doc.byId.props.raw).toBe("alias:: book\ntags:: blah");
      expect(doc.byId[roots[1]].raw).toBe("");
      expect(doc.byId[roots[1]].parent).toBeNull();
      expect(doc.byId.props.children).toEqual([]);
      expect(editingId()).toBe(roots[1]);
    } finally {
      dispose();
    }
  });

  it("does not reinterpret a later property-looking block as page properties", () => {
    loadSingle(page([block("body", "Reading list"), block("ordinary", "tags:: blah")]));
    startEditing("ordinary", "tags:: blah".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Properties")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      pressEnter(editorIn(root));
      expect(pageByName("Properties")!.roots).toHaveLength(3);
    } finally {
      dispose();
    }
  });
});

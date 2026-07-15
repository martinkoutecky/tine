import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import { initParser } from "../render/parse";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(() => initParser());

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(node, root) };
}

function page(raw: string): PageDto {
  const block: BlockDto = { id: "reference-authoring", raw, collapsed: false, children: [] };
  return { name: "Reference authoring", kind: "page", title: "Reference authoring", pre_block: null, blocks: [block] };
}

function update(textarea: HTMLTextAreaElement) {
  textarea.focus();
  textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: textarea.value.at(-1) }));
}

function accept(textarea: HTMLTextAreaElement, key: "Enter" | "Tab" = "Enter") {
  textarea.dispatchEvent(new KeyboardEvent("keydown", { key, code: key, bubbles: true, cancelable: true }));
}

describe("reference authoring", () => {
  it("makes Page reference the active bare slash command and chains into a blank page lifecycle", async () => {
    loadSingle(page("/"));
    startEditing("reference-authoring", 1);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      update(textarea);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Page reference"));
      accept(textarea);
      await vi.waitFor(() => expect(doc.byId["reference-authoring"].raw).toBe("[[]]"));
      expect(textarea.selectionStart).toBe(2);
      expect(textarea.selectionEnd).toBe(2);
      expect(document.body.querySelector(".autocomplete")).toBeNull();
    } finally {
      dispose();
    }
  });
});

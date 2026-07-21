import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(() => initParser());

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(node, root) };
}

function page(raw: string): PageDto {
  const block: BlockDto = { id: "property-authoring", raw, collapsed: false, children: [] };
  return { name: "Property authoring", kind: "page", title: "Property authoring", pre_block: null, blocks: [block] };
}

function inputAt(textarea: HTMLTextAreaElement, value: string, caret = value.length) {
  textarea.focus();
  textarea.value = value;
  textarea.setSelectionRange(caret, caret);
  textarea.dispatchEvent(new InputEvent("input", {
    bubbles: true,
    inputType: "insertText",
    data: value[caret - 1] ?? null,
  }));
}

function enter(textarea: HTMLTextAreaElement) {
  textarea.dispatchEvent(new KeyboardEvent("keydown", {
    key: "Enter", code: "Enter", bubbles: true, cancelable: true,
  }));
}

describe("property name/value autocomplete", () => {
  it("replaces only the property span, then offers values for the canonical key", async () => {
    const facets = vi.spyOn(backend(), "queryFacets").mockResolvedValue([
      ["alpha", ["one", "two"]],
      ["alpha-value", ["folded"]],
      ["template", ["My template"]],
      ["title", ["Page title"]],
    ]);
    loadSingle(page("prefix\nalp:: suffix"));
    startEditing("property-authoring", "prefix\nalp::".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Property authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "prefix\nalp:: suffix", "prefix\nalp::".length);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("alpha"));
      expect(facets).toHaveBeenCalledWith(true);

      enter(textarea);
      await vi.waitFor(() => expect(textarea.value).toBe("prefix\nalpha::  suffix"));
      expect(textarea.selectionStart).toBe("prefix\nalpha:: ".length);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("one"));

      enter(textarea);
      await vi.waitFor(() => expect(doc.byId["property-authoring"].raw).toBe("prefix\nalpha:: one suffix"));
    } finally {
      dispose();
    }
  });

  it("does not trigger in prose or a fenced code block", async () => {
    const facets = vi.spyOn(backend(), "queryFacets").mockResolvedValue([["alpha", ["one"]]]);
    loadSingle(page("ordinary prose ::\n```\n::"));
    startEditing("property-authoring", "ordinary prose ::".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Property authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, textarea.value, "ordinary prose ::".length);
      await new Promise((resolve) => setTimeout(resolve, 120));
      expect(document.body.querySelector(".autocomplete")).toBeNull();

      inputAt(textarea, textarea.value, textarea.value.length);
      await new Promise((resolve) => setTimeout(resolve, 120));
      expect(document.body.querySelector(".autocomplete")).toBeNull();
      expect(facets).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });
});

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
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
  it("keeps the OG line-opening :: workflow live through name and comma-separated values (GH #306)", async () => {
    vi.spyOn(backend(), "queryFacets").mockResolvedValue([
      ["alpha", ["one", "two"]],
      ["beta", ["three"]],
    ]);
    loadSingle(page(""));
    startEditing("property-authoring", 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Property authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "::", 2);
      await vi.waitFor(() => expect(textarea.selectionStart).toBe(0));
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("alpha"));

      inputAt(textarea, "alp::", 3);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe('Create "alp"'));
      expect([...document.body.querySelectorAll(".autocomplete .ac-label")].map((el) => el.textContent))
        .toEqual(['Create "alp"', "alpha"]);

      inputAt(textarea, "alpha::", 5);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("alpha"));
      enter(textarea);
      await vi.waitFor(() => expect(textarea.value).toBe("alpha:: "));
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("one"));

      enter(textarea);
      await vi.waitFor(() => expect(textarea.value).toBe("alpha:: one"));
      inputAt(textarea, "alpha:: one,", "alpha:: one,".length);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("two"));
      expect(document.body.textContent).not.toContain("one");
    } finally {
      dispose();
    }
  });

  it("opens value suggestions when a directly-authored property is followed by a space (GH #306)", async () => {
    vi.spyOn(backend(), "queryFacets").mockResolvedValue([["alpha", ["one"]]]);
    loadSingle(page(""));
    startEditing("property-authoring", 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Property authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "alpha::", "alpha::".length);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("alpha"));
      inputAt(textarea, "alpha:: ", "alpha:: ".length);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("one"));
    } finally {
      dispose();
    }
  });

  it("replaces only the property span, then offers values for the canonical key", async () => {
    const facets = vi.spyOn(backend(), "queryFacets").mockResolvedValue([
      ["alpha", ["one", "two"]],
      ["alpha-value", ["folded"]],
      ["template", ["My template"]],
      ["title", ["Page title"]],
    ]);
    loadSingle(page("prefix\nalpha:: suffix"));
    startEditing("property-authoring", "prefix\nalpha::".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Property authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "prefix\nalpha:: suffix", "prefix\nalpha::".length);
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

  it("a failed facet query leaves the editor usable: no rejection, no toast, no popup", async () => {
    const facets = vi.spyOn(backend(), "queryFacets").mockRejectedValue(new Error("facet index unavailable"));
    const rejected: unknown[] = [];
    const onRejection = (event: PromiseRejectionEvent) => { rejected.push(event.reason); event.preventDefault(); };
    window.addEventListener("unhandledrejection", onRejection);
    loadSingle(page(""));
    startEditing("property-authoring", 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Property authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "alp::", 3);
      await vi.waitFor(() => expect(facets).toHaveBeenCalled());
      await new Promise((resolve) => setTimeout(resolve, 60));
      expect(rejected).toEqual([]);
      expect(document.body.querySelector(".toast")).toBeNull();
      expect(document.body.querySelector(".autocomplete .ac-item")).toBeNull();
      // The typed text is untouched and still editable.
      expect(textarea.value).toBe("alp::");
    } finally {
      window.removeEventListener("unhandledrejection", onRejection);
      dispose();
    }
  });
});

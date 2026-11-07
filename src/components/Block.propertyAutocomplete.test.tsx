import { editorIn, type EditorDriver } from "../tests/editorDriver";
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

function inputAt(textarea: EditorDriver, value: string, caret = value.length) {
  textarea.focus();
  textarea.input(value, caret, {
    inputType: "insertText",
    data: value[caret - 1] ?? null,
  });
}

function enter(textarea: EditorDriver) {
  textarea.keydown({
    key: "Enter", code: "Enter", bubbles: true, cancelable: true,
  });
}

describe("property name/value autocomplete", () => {
  // Intentional OG difference: value search also starts without choosing a name
  // (OG 6e7afa8eb editor.cljs:2211-2226), for every Markdown entry method.
  it.each(["typed", "pasted", "existing"])("offers values on a %s property without a prior name session", async method => {
    const facets = vi.spyOn(backend(), "queryFacets").mockResolvedValue([["alpha", ["one", "two"]]]);
    const raw = method === "existing" ? "Body\nalpha:: o" : "Body\n";
    loadSingle(page(raw));
    startEditing("property-authoring", raw.length);
    const { root, dispose } = mount(() => <Block id="property-authoring" />);
    try {
      const editor = editorIn(root);
      editor.focus();
      if (method === "typed") {
        for (const ch of "alpha:: ") editor.type(ch);
      } else if (method === "pasted") {
        const event = editor.paste({ text: "alpha:: " });
        // jsdom has no browser paste default; report its resulting input when
        // the editor deliberately leaves a plain single-line paste to it.
        if (!event.defaultPrevented) editor.input("Body\nalpha:: ", 13, { inputType: "insertFromPaste", data: null });
      } else {
        editor.type("n");
      }
      await vi.waitFor(() => expect(document.querySelector(".autocomplete .ac-label")?.textContent).toBe("one"));
      enter(editor);
      await vi.waitFor(() => expect(editor.value()).toBe("Body\nalpha:: one"));
      expect(facets).toHaveBeenCalledWith(true, { key: "alpha" });
    } finally { dispose(); }
  });

  it("does not open value suggestions when the caret only moves onto an existing value", async () => {
    const facets = vi.spyOn(backend(), "queryFacets").mockResolvedValue([["alpha", ["one", "two"]]]);
    const raw = "Body\nalpha:: o";
    loadSingle(page(raw));
    startEditing("property-authoring", 0);
    const { root, dispose } = mount(() => <Block id="property-authoring" />);
    try {
      const editor = editorIn(root);
      editor.focus();
      editor.selectByUser(raw.length);
      await new Promise(resolve => setTimeout(resolve, 250));
      expect(document.querySelector(".autocomplete .ac-label")).toBeNull();
      expect(facets).not.toHaveBeenCalledWith(true, { key: "alpha" });
    } finally { dispose(); }
  });

  it("updates the current value query when pending values arrive, without another keystroke", async () => {
    let finish!: (values: [string, string[]][]) => void;
    const facets = vi.spyOn(backend(), "queryFacets").mockImplementation((_autocomplete, scope) => {
      if (scope && "keys" in scope) return Promise.resolve([["alpha", []]]);
      return new Promise(resolve => { finish = resolve; });
    });
    loadSingle(page("Body\n")); startEditing("property-authoring", 5);
    const { root, dispose } = mount(() => <Block id="property-authoring" />);
    try {
      const editor = editorIn(root);
      // Start with the formerly working name -> value path to isolate arrival
      // from the separate missing-name-session bug.
      inputAt(editor, "Body\nalpha::", 10);
      await vi.waitFor(() => expect(document.querySelector(".autocomplete .ac-label")?.textContent).toBe("alpha"));
      enter(editor);
      await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
      editor.type("t");
      await vi.waitFor(() => expect(document.querySelector(".autocomplete .ac-label")?.textContent).toBe('Create "t"'));
      finish([["alpha", ["one", "two"]]]);
      await vi.waitFor(() => expect(document.querySelector(".autocomplete .ac-label")?.textContent).toBe("two"));
      expect(facets.mock.calls.filter(([, scope]) => scope && "key" in scope)).toHaveLength(1);
    } finally { dispose(); }
  });

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
      const textarea = editorIn(root);
      inputAt(textarea, "::", 2);
      await vi.waitFor(() => expect(textarea.selection()[0]).toBe(0));
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("alpha"));

      inputAt(textarea, "alp::", 3);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe('Create "alp"'));
      expect([...document.body.querySelectorAll(".autocomplete .ac-label")].map((el) => el.textContent))
        .toEqual(['Create "alp"', "alpha"]);

      inputAt(textarea, "alpha::", 5);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("alpha"));
      enter(textarea);
      await vi.waitFor(() => expect(textarea.value()).toBe("alpha:: "));
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("one"));

      enter(textarea);
      await vi.waitFor(() => expect(textarea.value()).toBe("alpha:: one"));
      inputAt(textarea, "alpha:: one,", "alpha:: one,".length);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("two"));
      expect(document.querySelector(".autocomplete")?.textContent).not.toContain("one");
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
      const textarea = editorIn(root);
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
      const textarea = editorIn(root);
      inputAt(textarea, "prefix\nalpha:: suffix", "prefix\nalpha::".length);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("alpha"));
      expect(facets).toHaveBeenCalledWith(true, { keys: true });

      enter(textarea);
      await vi.waitFor(() => expect(textarea.value()).toBe("prefix\nalpha::  suffix"));
      expect(textarea.selection()[0]).toBe("prefix\nalpha:: ".length);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("one"));

      enter(textarea);
      await vi.waitFor(() => expect(doc.byId["property-authoring"].raw).toBe("prefix\nalpha:: one suffix"));
    } finally {
      dispose();
    }
  });

  it("fetches property keys once per completion session and values only for the chosen key (cost-model audit row 2b)", async () => {
    const facets = vi.spyOn(backend(), "queryFacets").mockImplementation(async (_autocomplete, scope) => {
      const all: [string, string[]][] = [["alpha", ["one", "two"]], ["beta", ["three"]]];
      if (scope && "key" in scope) return all.filter(([key]) => key === scope.key);
      if (scope && "keys" in scope) return all.map(([key]) => [key, []]);
      return all;
    });
    loadSingle(page(""));
    startEditing("property-authoring", 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Property authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = editorIn(root);
      inputAt(textarea, "a::", 1);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe('Create "a"'));
      for (const typed of ["al::", "alp::", "alph::"]) {
        inputAt(textarea, typed, typed.length - 2);
        await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe(`Create "${typed.slice(0, -2)}"`));
      }
      inputAt(textarea, "alpha::", 5);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("alpha"));
      enter(textarea);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("one"));
      // Five keystrokes of one session: one keys request (the old editor
      // fetched every key's values on each of them), then the chosen key's
      // values, and never every key's values.
      expect(facets.mock.calls).toEqual([[true, { keys: true }], [true, { key: "alpha" }]]);
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
      const textarea = editorIn(root);
      inputAt(textarea, textarea.value(), "ordinary prose ::".length);
      await new Promise((resolve) => setTimeout(resolve, 120));
      expect(document.body.querySelector(".autocomplete")).toBeNull();

      inputAt(textarea, textarea.value(), textarea.value().length);
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
      const textarea = editorIn(root);
      inputAt(textarea, "alp::", 3);
      await vi.waitFor(() => expect(facets).toHaveBeenCalled());
      await new Promise((resolve) => setTimeout(resolve, 60));
      expect(rejected).toEqual([]);
      expect(document.body.querySelector(".toast")).toBeNull();
      expect(document.body.querySelector(".autocomplete .ac-item")).toBeNull();
      // The typed text is untouched and still editable.
      expect(textarea.value()).toBe("alp::");
    } finally {
      window.removeEventListener("unhandledrejection", onRejection);
      dispose();
    }
  });
});

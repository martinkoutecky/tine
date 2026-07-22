import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { startEditing } from "../editorController";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import { initParser } from "../render/parse";
import type { BlockDto, PageDto, PageEntry } from "../types";
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
  const block: BlockDto = { id: "reference-authoring", raw, collapsed: false, children: [] };
  return { name: "Reference authoring", kind: "page", title: "Reference authoring", pre_block: null, blocks: [block] };
}

function update(textarea: HTMLTextAreaElement) {
  textarea.focus();
  textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: textarea.value.at(-1) }));
}

function inputAt(textarea: HTMLTextAreaElement, value: string, caret: number) {
  textarea.focus();
  textarea.value = value;
  textarea.setSelectionRange(caret, caret);
  textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value[caret - 1] ?? null }));
}

function entry(name: string): PageEntry {
  return { name, kind: "page", date_key: null, path: `pages/${name}.md` };
}

function accept(textarea: HTMLTextAreaElement, key: "Enter" | "Tab" = "Enter") {
  textarea.dispatchEvent(new KeyboardEvent("keydown", { key, code: key, bubbles: true, cancelable: true }));
}

describe("reference authoring", () => {
  it("renders PDF annotation navigation with the authored highlight id", () => {
    const authoredId = "7b6704f8-a337-4336-a711-2ba6bc14fbf2";
    loadSingle({
      name: "hls__paper",
      kind: "page",
      title: "hls__paper",
      pre_block: "file-path:: ../assets/paper.pdf",
      blocks: [{
        id: "runtime-annotation-id",
        raw: `Highlighted text\nid:: ${authoredId}\nls-type:: annotation\nhl-page:: 2\nhl-color:: yellow`,
        collapsed: false,
        children: [],
        properties: [
          ["id", authoredId],
          ["ls-type", "annotation"],
          ["hl-page", "2"],
          ["hl-color", "yellow"],
        ],
      }],
    });
    const { root, dispose } = mount(() => (
      <For each={pageByName("hls__paper")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      expect(root.querySelector(".hl-prefix")?.getAttribute("data-highlight-id")).toBe(authoredId);
    } finally {
      dispose();
    }
  });

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

  it("accepts a visible page row against the current trigger span before the debounced refresh", async () => {
    const quickSwitch = vi.spyOn(backend(), "quickSwitch").mockResolvedValue([entry("Parity Target")]);
    loadSingle(page("[[P]]"));
    startEditing("reference-authoring", 3);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "[[P]]", 3);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Parity Target"));
      expect(quickSwitch).toHaveBeenLastCalledWith("P", 100);

      // Type the rest and accept immediately, before the 90 ms backend debounce.
      // The visible row is still legitimate, but replacement must use the live
      // `[[Parity Tar` span rather than the stale `[[P` span.
      inputAt(textarea, "[[Parity Tar]]", 12);
      accept(textarea);

      await vi.waitFor(() => expect(doc.byId["reference-authoring"].raw).toBe("[[Parity Target]] "));
      expect(textarea.value).toBe("[[Parity Target]] ");
      expect(document.body.querySelector(".autocomplete")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("requests OG's 20-result inline block-reference pool", async () => {
    const search = vi.spyOn(backend(), "search").mockResolvedValue([{
      page: "Source",
      kind: "page",
      blocks: [{ id: "needle-block", raw: "needle", collapsed: false, children: [] }],
      evidence: [],
    }]);
    loadSingle(page("((needle))"));
    startEditing("reference-authoring", 8);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "((needle))", 8);
      await vi.waitFor(() => expect(search).toHaveBeenCalled());
      expect(search).toHaveBeenLastCalledWith("needle", 20, "block-picker");
    } finally {
      dispose();
    }
  });

  const blockReferenceCases: Array<[string, [string, string][] | undefined, string]> = [
    ["authored id", [["id", "11111111-1111-4111-8111-111111111111"]], "11111111-1111-4111-8111-111111111111"],
    ["id-less fallback", undefined, "f8358fac-56bd-8bb1-ba45-bd7fd1ba2add"],
  ];

  it.each(blockReferenceCases)("inserts the %s for an accepted block-reference suggestion", async (_case, properties, expectedId) => {
    vi.spyOn(backend(), "search").mockResolvedValue([{
      page: "Source",
      kind: "page",
      blocks: [{
        id: "f8358fac-56bd-8bb1-ba45-bd7fd1ba2add",
        raw: "test",
        collapsed: false,
        children: [],
        ...(properties ? { properties } : {}),
      }],
      evidence: [],
    }]);
    loadSingle(page("((test"));
    startEditing("reference-authoring", 6);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "((test", 6);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("test"));
      accept(textarea);
      await vi.waitFor(() => expect(doc.byId["reference-authoring"].raw).toBe(`((${expectedId})) `));
    } finally {
      dispose();
    }
  });

  it("does not let an older same-location page lookup overwrite newer results", async () => {
    const pending: { query: string; resolve: (pages: PageEntry[]) => void }[] = [];
    vi.spyOn(backend(), "quickSwitch").mockImplementation((query) =>
      new Promise<PageEntry[]>((resolve) => pending.push({ query, resolve }))
    );
    loadSingle(page("[[P]]"));
    startEditing("reference-authoring", 3);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Reference authoring")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "[[P]]", 3);
      await vi.waitFor(() => expect(pending.map((request) => request.query)).toEqual(["P"]));

      inputAt(textarea, "[[Parity]]", 8);
      await vi.waitFor(() => expect(pending.map((request) => request.query)).toEqual(["P", "Parity"]));

      pending[1].resolve([entry("Parity New")]);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Parity New"));
      pending[0].resolve([entry("Parity Old")]);
      await Promise.resolve();
      await Promise.resolve();
      expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Parity New");
    } finally {
      dispose();
    }
  });
});

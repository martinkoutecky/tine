import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function page(raw: string): PageDto {
  const block: BlockDto = { id: "advanced", raw, collapsed: false, children: [] };
  return {
    name: "Advanced commands",
    kind: "page",
    title: "Advanced commands",
    pre_block: null,
    format: "org",
    blocks: [block],
  };
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

function choose(label: string) {
  const item = [...document.body.querySelectorAll<HTMLElement>(".autocomplete .ac-item")]
    .find((candidate) => candidate.querySelector(".ac-label")?.textContent === label);
  expect(item).toBeDefined();
  item!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
}

describe("< advanced-section command autocomplete", () => {
  it("filters Quote/Query, inserts an Org BEGIN/END section at the middle caret, and preserves a second section", async () => {
    loadSingle(page("<qu"));
    startEditing("advanced", 3);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Advanced commands")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "<qu");
      await vi.waitFor(() => {
        const labels = [...document.body.querySelectorAll(".autocomplete .ac-label")]
          .map((element) => element.textContent);
        expect(labels).toEqual(["Quote", "Query"]);
      });

      choose("Quote");
      await vi.waitFor(() => expect(doc.byId.advanced.raw).toBe("#+BEGIN_QUOTE\n\n#+END_QUOTE"));
      expect(textarea.selectionStart).toBe("#+BEGIN_QUOTE\n".length);
      expect(textarea.selectionEnd).toBe("#+BEGIN_QUOTE\n".length);

      const second = "#+BEGIN_QUOTE\n\n#+END_QUOTE\n<ex";
      inputAt(textarea, second);
      await vi.waitFor(() => {
        const labels = [...document.body.querySelectorAll(".autocomplete .ac-label")]
          .map((element) => element.textContent);
        expect(labels).toContain("Example");
      });
      choose("Example");

      const expected = "#+BEGIN_QUOTE\n\n#+END_QUOTE\n#+BEGIN_EXAMPLE\n\n#+END_EXAMPLE";
      await vi.waitFor(() => expect(doc.byId.advanced.raw).toBe(expected));
      expect(textarea.selectionStart).toBe(expected.indexOf("\n\n#+END_EXAMPLE") + 1);
      expect(textarea.selectionEnd).toBe(expected.indexOf("\n\n#+END_EXAMPLE") + 1);
    } finally {
      dispose();
    }
  });
});

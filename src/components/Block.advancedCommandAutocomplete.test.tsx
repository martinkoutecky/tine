import { editorIn, type EditorDriver } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";
import { setActiveLocale } from "../i18n";
import { ADVANCED_BLOCK_COMMANDS } from "../editor/autocomplete";

beforeAll(async () => {
  await initParser();
});

afterEach(async () => {
  await setActiveLocale("en");
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

function inputAt(textarea: EditorDriver, value: string, caret = value.length) {
  textarea.focus();
  textarea.input(value, caret, {
    inputType: "insertText",
    data: value[caret - 1] ?? null,
  });
}

function choose(label: string) {
  const item = [...document.body.querySelectorAll<HTMLElement>(".autocomplete .ac-item")]
    .find((candidate) => candidate.querySelector(".ac-label")?.textContent === label);
  expect(item).toBeDefined();
  item!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
}

describe("< advanced-section command autocomplete", () => {
  it("finds Quote from its Chinese label and inserts the original Org section (GH #107)", async () => {
    await setActiveLocale("zh-CN");
    loadSingle(page("<引用"));
    startEditing("advanced", 3);
    const { root, dispose } = mount(() => <Block id="advanced" />);
    try {
      inputAt(editorIn(root), "<引用");
      const quote = ADVANCED_BLOCK_COMMANDS.find(command => command.matchLabel === "Quote")!;
      await vi.waitFor(() => expect([...document.querySelectorAll(".autocomplete .ac-label")]
        .map(el => el.textContent)).toContain(quote.label));
      choose(quote.label);
      await vi.waitFor(() => expect(doc.byId.advanced.raw).toBe(quote.insert));
    } finally { dispose(); }
  });
  it("filters Quote/Query, inserts an Org BEGIN/END section at the middle caret, and preserves a second section", async () => {
    loadSingle(page("<qu"));
    startEditing("advanced", 3);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Advanced commands")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = editorIn(root);
      inputAt(textarea, "<qu");
      await vi.waitFor(() => {
        const labels = [...document.body.querySelectorAll(".autocomplete .ac-label")]
          .map((element) => element.textContent);
        expect(labels).toEqual(["Quote", "Query"]);
      });

      choose("Quote");
      await vi.waitFor(() => expect(doc.byId.advanced.raw).toBe("#+BEGIN_QUOTE\n\n#+END_QUOTE"));
      expect(textarea.selection()[0]).toBe("#+BEGIN_QUOTE\n".length);
      expect(textarea.selection()[1]).toBe("#+BEGIN_QUOTE\n".length);

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
      expect(textarea.selection()[0]).toBe(expected.indexOf("\n\n#+END_EXAMPLE") + 1);
      expect(textarea.selection()[1]).toBe(expected.indexOf("\n\n#+END_EXAMPLE") + 1);
    } finally {
      dispose();
    }
  });
});

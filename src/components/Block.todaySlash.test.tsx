import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import { formatJournal, setJournalTitleFormat } from "../journal";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  setJournalTitleFormat(null); // module-level state — restore the default format
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function page(raw: string): PageDto {
  const block: BlockDto = { id: "today-host", raw, collapsed: false, children: [] };
  return {
    name: "Slash today",
    kind: "page",
    title: "Slash today",
    pre_block: null,
    format: "md",
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

describe("/Today slash command (GH #220)", () => {
  it("inserts the day link in the graph's configured journal title format", async () => {
    setJournalTitleFormat("yyyy-MM-dd");
    loadSingle(page("/today"));
    startEditing("today-host", 6);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Slash today")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "/today");
      await vi.waitFor(() => {
        const labels = [...document.body.querySelectorAll(".autocomplete .ac-label")]
          .map((element) => element.textContent);
        expect(labels).toContain("Today");
      });

      choose("Today");

      const expected = `[[${formatJournal(new Date(), "yyyy-MM-dd")}]]`;
      // trailing space comes from the space-after-ref completion default (GH #35)
      await vi.waitFor(() => expect(doc.byId["today-host"].raw.trimEnd()).toBe(expected));
    } finally {
      dispose();
    }
  });
});

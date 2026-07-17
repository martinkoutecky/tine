import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { loadSingle, pageByName, resetStore } from "../store";
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
  const block: BlockDto = { id: "tag-ime", raw, collapsed: false, children: [] };
  return { name: "Tag IME", kind: "page", title: "Tag IME", pre_block: null, blocks: [block] };
}

describe("bare hashtag autocomplete after IME input (GH #167)", () => {
  it("shows tag choices for a committed CJK prefix at the visible editor boundary", async () => {
    loadSingle(page("#倘"));
    startEditing("tag-ime", "#倘".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Tag IME")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      textarea.focus();
      textarea.setSelectionRange(textarea.value.length, textarea.value.length);
      textarea.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data: "" }));
      textarea.dispatchEvent(new InputEvent("input", {
        bubbles: true,
        inputType: "insertCompositionText",
        data: "倘",
        isComposing: true,
      }));
      textarea.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "倘" }));

      await vi.waitFor(() => {
        expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Create #倘");
      });
    } finally {
      dispose();
    }
  });
});

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import { startEditing } from "../editorController";
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
  const block: BlockDto = { id: "code", raw, collapsed: false, children: [] };
  return { name: "Code", kind: "page", title: "Code", pre_block: null, blocks: [block] };
}

function openCompletion(textarea: HTMLTextAreaElement) {
  textarea.focus();
  textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  textarea.dispatchEvent(new InputEvent("input", {
    bubbles: true,
    inputType: "insertText",
    data: textarea.value.at(-1) ?? null,
  }));
}

function enter(textarea: HTMLTextAreaElement) {
  textarea.dispatchEvent(new KeyboardEvent("keydown", {
    key: "Enter",
    bubbles: true,
    cancelable: true,
  }));
}

describe("fenced-code language completion (GH #94)", () => {
  it("completes a typed alias to the bundled canonical language id", async () => {
    loadSingle(page("```js"));
    startEditing("code", 5);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      openCompletion(textarea);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-item")).not.toBeNull());
      expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("JavaScript");

      enter(textarea);
      await vi.waitFor(() => expect(doc.byId.code.raw).toBe("```javascript"));
    } finally {
      dispose();
    }
  });

  it("opens an empty language picker from /Code block and keeps a complete scaffold", async () => {
    loadSingle(page("/code"));
    startEditing("code", "/code".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      openCompletion(textarea);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-item")).not.toBeNull());
      expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Code block");

      enter(textarea);
      await vi.waitFor(() => expect(doc.byId.code.raw).toBe("```\n\n```"));
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("JavaScript"));

      enter(textarea);
      await vi.waitFor(() => expect(doc.byId.code.raw).toBe("```javascript\n\n```"));
      expect(textarea.selectionStart).toBe("```javascript\n".length);
    } finally {
      dispose();
    }
  });
});

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
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
      // The language picked from the scaffold's opener line completes the
      // wrapper, swapping to the body-only code view: the caret lands at the
      // start of the (empty) payload — the code-line position in body space.
      expect(textarea.selectionStart).toBe(0);
      expect(textarea.value).not.toContain("```");
    } finally {
      dispose();
    }
  });

  it("keeps the scaffold's language picker typeable until a language completes the wrapper", async () => {
    loadSingle(page("/code"));
    startEditing("code", "/code".length);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Code")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      openCompletion(textarea);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-item")).not.toBeNull());
      enter(textarea);
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-item")).not.toBeNull());

      // Typing the query refines the picker on the still-raw scaffold (the
      // body-only swap waits for the picker so it can keep its opener line).
      textarea.setSelectionRange(3, 3);
      for (const ch of "javas") {
        const before = textarea.selectionStart;
        textarea.value = textarea.value.slice(0, before) + ch + textarea.value.slice(before);
        textarea.setSelectionRange(before + 1, before + 1);
        textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: ch }));
      }
      expect(doc.byId.code.raw).toBe("```javas\n\n```");
      await vi.waitFor(() => {
        const refined = [...document.body.querySelectorAll(".autocomplete .ac-item")];
        expect(refined.length).toBeGreaterThan(0);
        expect(refined.some((item) => item.querySelector(".ac-label")?.textContent === "JavaScript")).toBe(true);
      });
      // The scaffold's raw stays byte-stable while the query is being typed.
      expect(doc.byId.code.raw).toBe("```javas\n\n```");

      // Accept JavaScript: the canonical id lands on the opener and the view
      // swaps to the body-only payload.
      const pick = [...document.body.querySelectorAll(".autocomplete .ac-item")]
        .find((item) => item.querySelector(".ac-label")?.textContent === "JavaScript")!;
      pick.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(doc.byId.code.raw).toBe("```javascript\n\n```"));
      expect(textarea.selectionStart).toBe(0);
      expect(textarea.value).not.toContain("```");
    } finally {
      dispose();
    }
  });
});

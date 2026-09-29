import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { editingId, endEdit } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore, selectedIds } from "../document";
import { loadSingle } from "../document/workingSet";
import type { PageDto } from "../types";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

const page: PageDto = {
  name: "Mouse down",
  kind: "page",
  title: "Mouse down",
  pre_block: null,
  blocks: [
    { id: "mouse-down-edit", raw: "Caret now", collapsed: false, children: [] },
    { id: "mouse-down-other", raw: "Another block", collapsed: false, children: [] },
  ],
};

function mount() {
  loadSingle(page);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(
    () => <For each={pageByName(page.name)?.roots ?? []}>{(id) => <Block id={id} />}</For>,
    host,
  );
  return { host, dispose };
}

const mouse = (type: string, x: number, y: number) =>
  new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: x, clientY: y });

describe("rendered block edit gesture (GH #368)", () => {
  it("mounts and focuses the editor on primary mousedown, before mouseup", () => {
    const { host, dispose } = mount();
    try {
      const content = host.querySelector<HTMLElement>('[data-block-id="mouse-down-edit"] .block-content')!;
      const down = mouse("mousedown", 1, 1);
      content.dispatchEvent(down);

      const editor = host.querySelector<HTMLTextAreaElement>("textarea.block-editor");
      expect(down.defaultPrevented).toBe(true);
      expect(editingId()).toBe("mouse-down-edit");
      expect(editor).not.toBeNull();
      expect(document.activeElement).toBe(editor);
      document.dispatchEvent(mouse("mouseup", 1, 1));
    } finally {
      dispose();
    }
  });

  it("still escalates a drag that enters another block to block selection and leaves edit mode", () => {
    const { host, dispose } = mount();
    try {
      const content = host.querySelector<HTMLElement>('[data-block-id="mouse-down-edit"] .block-content')!;
      const other = host.querySelector<HTMLElement>('[data-block-id="mouse-down-other"] .block-content')!;
      content.dispatchEvent(mouse("mousedown", 1, 1));
      expect(editingId()).toBe("mouse-down-edit");

      // jsdom has no layout: answer "which element is under the pointer" directly.
      (document as Document & { elementFromPoint: (x: number, y: number) => Element | null }).elementFromPoint =
        () => other;
      document.dispatchEvent(mouse("mousemove", 1, 60));

      expect(editingId()).toBeNull();
      expect(selectedIds()).toEqual(["mouse-down-edit", "mouse-down-other"]);
      document.dispatchEvent(mouse("mouseup", 1, 60));
    } finally {
      delete (document as Partial<Document>).elementFromPoint;
      dispose();
    }
  });
});

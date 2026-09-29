// GH #279: Mod+Shift+C copies `{{embed ((uuid))}}` for the current block,
// mirroring the builtin Mod+C block-ref copy. With a live text selection the
// handler must decline (no preventDefault, no clipboard write) so the
// platform's ordinary copy keeps its meaning.
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { pageByName } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { installKeybindings } from "../keybindings";
import { Block } from "./Block";

const UUID = "8a67e2b1-70d7-4878-961b-c17dc3dc78bf";

const targetBlock: BlockDto = {
  id: "embed-target",
  raw: `Target block\nid:: ${UUID}`,
  collapsed: false,
  children: [],
};

function page(blocks: BlockDto[]): PageDto {
  return { name: "EmbedCopy", kind: "page", title: "EmbedCopy", pre_block: null, blocks };
}

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function mountEditing() {
  loadSingle(page([targetBlock]));
  startEditing("embed-target", 0);
  return mount(() => (
    <For each={pageByName("EmbedCopy")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ));
}

function chord(ta: HTMLTextAreaElement, init: KeyboardEventInit): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { key: "c", bubbles: true, cancelable: true, ...init });
  ta.dispatchEvent(event);
  return event;
}

let disposeKeys: (() => void) | null = null;

beforeAll(async () => {
  await initParser();
  disposeKeys = installKeybindings();
});

afterAll(() => {
  disposeKeys?.();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

describe("copy block embed command (GH #279)", () => {
  it("copies {{embed ((uuid))}} for the current block when nothing is selected", async () => {
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
    const { root, dispose } = mountEditing();
    try {
      const ta = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      ta.setSelectionRange(2, 2);
      const event = chord(ta, { ctrlKey: true, shiftKey: true });
      await vi.waitFor(() => expect(writeText).toHaveBeenCalled());

      expect(event.defaultPrevented).toBe(true);
      expect(writeText).toHaveBeenCalledWith(`{{embed ((${UUID}))}}`);
    } finally {
      dispose();
    }
  });

  it("declines with a live selection: no preventDefault, no clipboard write", async () => {
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
    const { root, dispose } = mountEditing();
    try {
      const ta = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      ta.setSelectionRange(1, 4); // a live selection — ordinary copy must win
      const event = chord(ta, { ctrlKey: true, shiftKey: true });
      // Let any (wrongly scheduled) handler promise settle before asserting.
      await Promise.resolve();
      await Promise.resolve();

      expect(event.defaultPrevented).toBe(false);
      expect(writeText).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("keeps the sibling contract intact: Mod+C with no selection copies ((uuid))", async () => {
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
    const { root, dispose } = mountEditing();
    try {
      const ta = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      ta.setSelectionRange(2, 2);
      const event = chord(ta, { ctrlKey: true });
      await vi.waitFor(() => expect(writeText).toHaveBeenCalled());

      expect(event.defaultPrevented).toBe(true);
      expect(writeText).toHaveBeenCalledWith(`((${UUID}))`);
    } finally {
      dispose();
    }
  });
});

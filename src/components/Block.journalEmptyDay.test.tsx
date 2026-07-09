import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { doc, loadFeed, nextVisible, pageByName, resetStore } from "../store";
import { editingId, startEditing } from "../editorController";
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
  const dispose = render(node, root);
  return { root, dispose };
}

function blk(id: string, raw: string): BlockDto {
  return { id, raw, collapsed: false, children: [] };
}

function journal(name: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "journal", title: name, pre_block: null, blocks };
}

describe("journal feed empty-day editing", () => {
  it("keeps the day's only empty root when Backspace sees the next feed block on another day", () => {
    loadFeed([
      journal("Today", [blk("today-empty", "")]),
      journal("Yesterday", [blk("yesterday-empty", "")]),
    ]);
    const today = pageByName("Today")!;
    const yesterday = pageByName("Yesterday")!;
    const todayRoot = today.roots[0];
    expect(nextVisible(todayRoot)).toBe(yesterday.roots[0]);

    startEditing(todayRoot, 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("Today")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement | null;
      expect(textarea).not.toBeNull();
      textarea!.focus();
      textarea!.setSelectionRange(0, 0);
      textarea!.dispatchEvent(new KeyboardEvent("keydown", { key: "Backspace", bubbles: true, cancelable: true }));

      expect(pageByName("Today")!.roots).toEqual([todayRoot]);
      expect(doc.byId[todayRoot].raw).toBe("");
      expect(editingId()).toBe(todayRoot);
    } finally {
      dispose();
    }
  });
});

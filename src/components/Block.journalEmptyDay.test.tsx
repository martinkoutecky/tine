import { describe, expect, it } from "vitest";
import { For } from "solid-js";
import { loadFeed, nextVisible, pageByName } from "../document";
import { doc } from "../document/model";
import { editingId, startEditing } from "../editorController";
import { Block } from "./Block";
import { installBlockEditorLifecycle, mount, blk, journal } from "../tests/blockEditorTestkit";

installBlockEditorLifecycle();

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

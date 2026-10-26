// GH #208 / #338: the find panel's replace row, its keyboard-free door (the page
// header's "⋯" menu), and replace-all over a block that is open in an editor.
// UI-INPAGE-FIND-REPLACE-208, UI-INPAGE-FIND-MOBILE-ENTRY-338.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
// The keyboard-free page-menu entry belongs to mobile (UI census 2026-10).
vi.mock("./nativeChrome", async (original) => ({ ...(await original<typeof import("./nativeChrome")>()), isMobilePlatform: true }));
import { For, Show } from "solid-js";
import { loadSingle } from "./document/workingSet";
import { pageByName, undo } from "./document";
import { doc } from "./document/model";
import { editingId, startEditing } from "./editorController";
import { closeInPageFind, inPageFindMatches, openInPageFind, setInPageFindQuery } from "./inpageFind";
import { setInPageReplaceOpen, setInPageReplaceText } from "./inpageReplace";
import { resetPaneLayoutToSingle } from "./panes";
import { closeContextMenu, openPageContextMenu } from "./ui";
import { setToasts } from "./toasts";
import type { PaneSnapshot } from "./router";
import { Block } from "./components/Block";
import { ContextMenu } from "./components/ContextMenu";
import { InPageFind } from "./components/InPageFind";
import { blk, installBlockEditorLifecycle, mount, page } from "./tests/blockEditorTestkit";

installBlockEditorLifecycle();

const NAME = "Notes";
const pageSnapshot = (name: string): PaneSnapshot => ({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
const journalsSnapshot = (): PaneSnapshot => ({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
const raws = () => pageByName(NAME)!.roots.map((id) => doc.byId[id].raw);
const frame = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));

function typeInto(input: HTMLInputElement | HTMLTextAreaElement, value: string) {
  input.value = value;
  input.dispatchEvent(new InputEvent("input", { bubbles: true }));
}

beforeEach(() => {
  setToasts([]);
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  resetPaneLayoutToSingle(pageSnapshot(NAME));
});
afterEach(() => {
  setInPageFindQuery("");
  closeInPageFind({ restoreFocus: false });
  setInPageReplaceOpen(false);
  setInPageReplaceText("");
  closeContextMenu();
  resetPaneLayoutToSingle(journalsSnapshot());
  vi.restoreAllMocks();
});

describe("GH #338: Find opens without a keyboard", () => {
  it("the mobile page header menu opens Find, and its replace row replaces every match", async () => {
    loadSingle(page(NAME, [blk("a", "foo one"), blk("b", "two foo")]));
    const { root, dispose } = mount(() => <><ContextMenu /><InPageFind /></>);
    try {
      const trigger = document.createElement("button");
      document.body.appendChild(trigger);
      openPageContextMenu(10, 10, NAME, "page", true, trigger);
      await frame();
      const item = document.querySelector<HTMLButtonElement>('[data-page-action-id="find-in-page"]');
      expect(item?.textContent).toBe("Find in page");
      item!.click();
      await Promise.resolve();
      const find = root.querySelector<HTMLInputElement>(".inpage-find-input");
      expect(find).not.toBeNull();
      await vi.waitFor(() => expect(document.activeElement).toBe(find));

      const toggle = root.querySelector<HTMLButtonElement>(".inpage-replace-toggle")!;
      expect(toggle.getAttribute("aria-expanded")).toBe("false");
      expect(root.querySelector(".inpage-replace-input")).toBeNull();
      toggle.click();
      const replace = root.querySelector<HTMLInputElement>(".inpage-replace-input")!;
      expect(replace).not.toBeNull();
      expect(toggle.getAttribute("aria-expanded")).toBe("true");

      typeInto(find!, "foo");
      typeInto(replace, "bar");
      const replaceAll = [...root.querySelectorAll<HTMLButtonElement>(".inpage-replace-btn")].find((b) => b.textContent?.trim() === "Replace all")!;
      await vi.waitFor(() => expect(replaceAll.disabled).toBe(false)); // the find query's debounce
      replaceAll.click();
      expect(raws()).toEqual(["bar one", "two bar"]);
    } finally {
      dispose();
    }
  });

  it("Enter in the replace field replaces the active match only", async () => {
    loadSingle(page(NAME, [blk("a", "foo foo")]));
    const { root, dispose } = mount(() => <InPageFind />);
    try {
      openInPageFind();
      setInPageFindQuery("foo");
      setInPageReplaceOpen(true);
      await Promise.resolve();
      const replace = root.querySelector<HTMLInputElement>(".inpage-replace-input")!;
      typeInto(replace, "bar");
      replace.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      expect(raws()).toEqual(["bar foo"]);
    } finally {
      dispose();
    }
  });
});

describe("GH #208 data safety: a block open in an editor", () => {
  it("replace-all keeps what was typed in the open editor, shows the result there, and undoes as one step", async () => {
    loadSingle(page(NAME, [blk("a", "foo alpha"), blk("b", "foo beta")]));
    startEditing("b", 0);
    const { root, dispose } = mount(() => (
      <>
        <Show when={pageByName(NAME)} keyed>{(p) => <For each={p.roots}>{(id) => <Block id={id} />}</For>}</Show>
        <InPageFind />
      </>
    ));
    try {
      const editor = root.querySelector<HTMLTextAreaElement>("textarea")!;
      expect(editingId()).toBe("b");
      typeInto(editor, "foo beta typed");
      // Find takes focus: the editor stays open and commits what was typed.
      openInPageFind();
      editor.dispatchEvent(new FocusEvent("blur"));
      setInPageFindQuery("foo");
      expect(inPageFindMatches()).toHaveLength(2);

      setInPageReplaceText("bar");
      const { replaceAllInPageFindMatches } = await import("./inpageReplace");
      expect(replaceAllInPageFindMatches().replaced).toBe(2);
      expect(raws()).toEqual(["bar alpha", "bar beta typed"]);
      // The still-open editor shows the replaced text, and a later commit from
      // it (blur, more typing) cannot put the old text back.
      const live = root.querySelector<HTMLTextAreaElement>("textarea")!;
      expect(live.value).toBe("bar beta typed");
      closeInPageFind({ restoreFocus: false });
      live.dispatchEvent(new FocusEvent("blur"));
      expect(raws()).toEqual(["bar alpha", "bar beta typed"]);

      expect(undo()).toBe(true);
      expect(raws()).toEqual(["foo alpha", "foo beta typed"]);
    } finally {
      dispose();
    }
  });
});

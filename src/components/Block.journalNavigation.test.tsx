import { afterEach, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { Block, SurfaceContext } from "./Block";
import { editorIn } from "../tests/editorDriver";
import { editingId, editingOwner, editingSurface, endEdit, startEditing } from "../editorController";
import { pageByName, resetStore } from "../document";
import { doc } from "../document/model";
import { loadFeed, ensurePageLoaded, appendFeed } from "../document/workingSet";
import { setFeedExtender } from "../document/edits/moves";
import { installKeybindings } from "../keybindings";
import type { PageDto } from "../types";

const disposers: (() => void)[] = [];
const flush = () => new Promise<void>(resolve => queueMicrotask(resolve));
const page = (name: string, id: string, raw: string, kind: "page" | "journal" = "page"): PageDto =>
  ({ name, title: name, kind, format: "md", pre_block: null, blocks: [{ id, raw, collapsed: false, children: [] }] });

function click(el: Element) {
  el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, cancelable: true }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
}

afterEach(() => {
  for (const dispose of disposers.splice(0)) dispose();
  setFeedExtender(null);
  endEdit("page-navigation");
  resetStore();
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

it.each(["other block", "same block in sidebar", "return to original", "same editor request", "no newer click", "move down then click", "move up then click"] as const)(
  "journal navigation respects %s",
  async newer => {
    disposers.push(installKeybindings());
    const moveUp = newer === "move up then click";
    const sameBlock = newer === "same block in sidebar";
    loadFeed([
      ...(moveUp ? [page("Oct 27th, 2026", "previous", "previous day", "journal")] : []),
      page("Oct 26th, 2026", "a", "old journal", "journal"),
    ]);
    ensurePageLoaded(page("Other", "b", "new click"));
    let resolve!: (ok: boolean) => void;
    const pending = new Promise<boolean>(r => { resolve = r; });
    const extender = vi.fn(() => pending);
    setFeedExtender(extender);
    const host = document.createElement("div");
    document.body.append(host);
    disposers.push(render(() => <>
      <section data-surface="main"><SurfaceContext.Provider value="main">
        <For each={doc.feed}>{name => <For each={pageByName(name)?.roots}>{id => <Block id={id} />}</For>}</For>
      </SurfaceContext.Provider></section>
      <section data-surface="sidebar"><SurfaceContext.Provider value="sidebar">
        <Block id={sameBlock ? "a" : "b"} />
      </SurfaceContext.Provider></section>
    </>, host));
    const mainBlock = () => host.querySelector('[data-surface="main"] [data-block-id="a"]')!;
    click(mainBlock().querySelector(".block-content-wrapper")!);
    await flush();
    const old = editorIn(mainBlock());
    old.select(old.value().length);
    const moving = newer.startsWith("move ");
    const key = old.keydown({ key: moveUp ? "ArrowUp" : "ArrowDown", altKey: moving, shiftKey: moving, bubbles: true, cancelable: true });
    expect(key.defaultPrevented).toBe(true);
    expect(extender).toHaveBeenCalledTimes(moveUp ? 0 : 1);

    let expectedBlock = "c";
    let retained = old;
    if (newer === "same editor request") {
      // Retain the same mounted editor and identity tuple, but publish a newer intent.
      startEditing("a", 2, editingOwner(), editingSurface());
      expectedBlock = "a";
    } else if (newer !== "no newer click") {
      const side = host.querySelector('[data-surface="sidebar"] [data-block-id]')!;
      click(side.querySelector(".block-content-wrapper")!);
      await flush();
      expectedBlock = sameBlock ? "a" : "b";
      retained = editorIn(side);
      retained.selectByUser(2);
      if (newer === "return to original") {
        click(mainBlock().querySelector(".block-content-wrapper")!);
        await flush();
        expectedBlock = "a";
        retained = editorIn(mainBlock());
      }
    }
    if (newer !== "no newer click") {
      expect(editingId()).toBe(expectedBlock);
      expect(document.activeElement).toBe(retained.element);
    }
    const selection = retained.selection();
    expect(appendFeed([page("Oct 25th, 2026", "c", "loaded later", "journal")])).toEqual([]);
    resolve(true);
    await pending;
    await flush();
    await flush();
    // The move continuation restores after a frame; this is a completion boundary,
    // not a timing assertion about focus or navigation speed.
    if (moving) await new Promise<void>(r => requestAnimationFrame(() => requestAnimationFrame(() => r())));
    expect(editingId()).toBe(expectedBlock);
    if (newer === "no newer click") {
      const next = editorIn(host.querySelector('[data-block-id="c"]')!);
      expect(document.activeElement).toBe(next.element);
      expect(next.selection()).toEqual([11, 11]);
    } else {
      expect(document.activeElement).toBe(retained.element);
      expect(retained.element.isConnected).toBe(true);
      expect(retained.selection()).toEqual(selection);
    }
    expect(doc.byId.a.raw).toBe("old journal");
    expect(doc.byId.b.raw).toBe("new click");
  },
);

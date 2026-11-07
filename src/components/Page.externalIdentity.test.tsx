import { For } from "solid-js";
import { afterEach, expect, it } from "vitest";
import { pageByName } from "../document";
import { installPageContent, loadSingle } from "../document/workingSet";
import { editingId, startEditing } from "../editorController";
import { blk, installBlockEditorLifecycle, mount, page } from "../tests/blockEditorTestkit";
import { BlockList } from "./BlockList";
import { SurfaceContext } from "./Block";
import { editorIn } from "../tests/editorDriver";
import { setDoc } from "../document/model";

installBlockEditorLifecycle();
let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); dispose = undefined; });

function large() {
  return page("Large", [blk("first", "Visible first block"), blk("second", "Visible second block"),
    { ...blk("tail", "Folded offscreen outline"), collapsed: true,
      children: Array.from({ length: 400 }, (_, i) => blk(`tail-${i}`, `Synthetic ${i} ${"ordinary prose ".repeat(75)}`)) }]);
}

it.each(["main", "rs:page:Large", "pane:split"])("%s retains its large outline and selection through unrelated, offscreen and visible external writes", async surface => {
  const initial = large();
  loadSingle(initial);
  // An acknowledged earlier edit can leave older source-preservation metadata
  // behind; adopting the new disk baseline must not replace this same editor.
  setDoc("byId", "second", "loadedRaw", "Earlier loaded source");
  const mounted = mount(() => <SurfaceContext.Provider value={surface}><For each={pageByName("Large") ? [pageByName("Large")!] : []}>{p =>
    <div class="page-blocks"><BlockList ids={p.roots} /></div>
  }</For></SurfaceContext.Provider>);
  dispose = mounted.dispose;
  const root = mounted.root;
  const container = root.querySelector(".page-blocks")!;
  const rows = [...root.querySelectorAll(".ls-block")];
  const witness = root.querySelector('[data-block-id="first"]')!;
  let removed = 0, blanks = 0;
  const observer = new MutationObserver(records => {
    for (const record of records) for (const removedNode of record.removedNodes)
      if (rows.some(row => removedNode === row || removedNode.contains(row))) removed++;
    if (!root.querySelector(".ls-block")) blanks++;
  });
  observer.observe(root, { childList: true, subtree: true });
  startEditing("second", 5);
  await Promise.resolve();
  const editor = editorIn(root);
  editor.focus(); editor.select(3, 9);
  root.scrollTop = 420;
  installPageContent("Other", page("Other", [blk("other", "unrelated write")]));
  await Promise.resolve();
  expect(root.querySelector(".page-blocks")).toBe(container);
  const offscreen = large();
  offscreen.blocks[2].children[399].raw += " updated offscreen";
  offscreen.blocks[2].children.push(blk("new-tail", "A newly appended hidden block"));
  installPageContent("Large", offscreen);
  await Promise.resolve();
  expect(root.querySelector(".page-blocks"), "offscreen edit replaced the page container").toBe(container);
  expect(root.querySelector('[data-block-id="first"]')).toBe(witness);
  expect(editorIn(root).element, "offscreen edit remounted the editor").toBe(editor.element);
  expect(editor.selection()).toEqual([3, 9]);
  expect(editingId()).toBe("second");
  expect(root.scrollTop).toBe(420);
  const visible = structuredClone(offscreen);
  visible.blocks[0].raw = "Visible first block changed externally";
  installPageContent("Large", visible);
  await Promise.resolve();
  expect(root.textContent).toContain("Visible first block changed externally");
  expect(root.querySelector('[data-block-id="first"]')).toBe(witness);
  expect(editorIn(root).element).toBe(editor.element);
  expect(editor.selection()).toEqual([3, 9]);
  expect(rows.every(row => row.isConnected)).toBe(true);
  expect({ removed, blanks }).toEqual({ removed: 0, blanks: 0 });
  observer.disconnect();
});

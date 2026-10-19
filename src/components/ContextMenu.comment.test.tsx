// Margin dialogue (vision §3.7): right-clicking words selected in a block's
// rendered text offers "Comment on selection" in the block menu, the same
// comment as Ctrl/Cmd+R. The editor keeps its native menu; nothing is offered
// without a selection inside this block.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { ContextMenu } from "./ContextMenu";
import { Block } from "./Block";
import { backend } from "../backend";
import { blockRegions, initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { pageByName, resetStore, undo } from "../document";
import { openContextMenu, closeContextMenu } from "../ui";
import { setGraphMeta } from "../graphSession";
import { clearTransientLayersForTest } from "../transientLayers";
import { editingId, endEdit } from "../editorController";
import type { BlockDto } from "../types";
import { bindTestHost } from "../document/host/wiring.test.support";

const PAGE = "CommentMenu";
let dispose: (() => void) | undefined;

function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  const properties = blockRegions(raw, "md").properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, collapsed: false, children, ...(properties.length ? { properties } : {}) };
}

beforeAll(() => initParser());
beforeEach(async () => {
  await bindTestHost();
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
});
afterEach(() => {
  dispose?.(); dispose = undefined;
  endEdit("page-navigation");
  closeContextMenu(); resetStore(); setGraphMeta(null); clearTransientLayersForTest(); vi.restoreAllMocks();
  document.body.innerHTML = "";
  window.getSelection()?.removeAllRanges();
});

function mount(blocks: BlockDto[]): HTMLDivElement {
  loadSingle({ name: PAGE, kind: "page", title: PAGE, format: "md", pre_block: null, blocks });
  setGraphMeta({ root: "/fixture" } as any);
  const root = document.createElement("div");
  document.body.append(root);
  dispose = render((): JSX.Element => (
    <>
      <For each={pageByName(PAGE)?.roots ?? []}>{(id) => <Block id={id} />}</For>
      <ContextMenu />
    </>
  ), root);
  return root;
}

/** Select [from, to) of the rendered text node inside block `id`. */
function selectRendered(root: HTMLElement, id: string, phrase: string, nth = 0): void {
  const content = root.querySelector(`[data-block-id="${id}"] .block-content`)!;
  const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
  let seen = 0;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const text = n.textContent ?? "";
    for (let at = text.indexOf(phrase); at !== -1; at = text.indexOf(phrase, at + 1)) {
      if (seen++ !== nth) continue;
      const range = document.createRange();
      range.setStart(n, at);
      range.setEnd(n, at + phrase.length);
      const selection = window.getSelection()!;
      selection.removeAllRanges();
      selection.addRange(range);
      return;
    }
  }
  throw new Error(`no ${phrase} #${nth} in ${id}`);
}

const items = () => [...document.querySelectorAll<HTMLElement>(".ctx-item")].map((el) => el.textContent?.trim());
const commentItem = () => [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((el) => el.textContent?.trim() === "Comment on selection");

describe("block menu: Comment on selection", () => {
  it("comments on words selected in the rendered text, as Ctrl+R does", async () => {
    const root = mount([block("p", "An agent wrote a phrase worth disputing here."), block("q", "A second block stays.")]);
    selectRendered(root, "p", "a phrase worth disputing");
    openContextMenu(10, 10, "p");
    await vi.waitFor(() => expect(commentItem()).toBeDefined());
    commentItem()!.click();
    const parent = doc.byId["p"];
    expect(parent.children).toHaveLength(1);
    expect(doc.byId[parent.children[0]].raw).toBe("\nquote:: a phrase worth disputing\nquote-prefix:: An agent wrote\nquote-suffix:: here.");
    expect(editingId()).toBe(parent.children[0]);
    expect(doc.byId["q"].children).toHaveLength(0);
    undo();
    expect(doc.byId["p"].children).toHaveLength(0);
  });

  it("anchors a repeated phrase at the occurrence that was selected", async () => {
    const root = mount([block("p", "it is true that it is true.")]);
    selectRendered(root, "p", "it is true", 1);
    openContextMenu(10, 10, "p");
    await vi.waitFor(() => expect(commentItem()).toBeDefined());
    commentItem()!.click();
    const raw = doc.byId[doc.byId["p"].children[0]].raw;
    expect(raw).toContain("quote:: it is true");
    expect(raw).toContain("quote-prefix:: it is true that");
  });

  it("is absent without a selection inside this block", async () => {
    const root = mount([block("p", "First block text."), block("q", "Second block text.")]);
    openContextMenu(10, 10, "p");
    await vi.waitFor(() => expect(items().length).toBeGreaterThan(3));
    expect(commentItem()).toBeUndefined();
    closeContextMenu();
    selectRendered(root, "q", "Second");
    openContextMenu(10, 10, "p");
    await vi.waitFor(() => expect(items().length).toBeGreaterThan(3));
    expect(commentItem()).toBeUndefined();
  });
});

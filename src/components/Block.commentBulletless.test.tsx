// Margin dialogue, slice 2b: a comment and its replies draw no bullet and no
// fold arrow; the author chip keeps the block menu on long press (DESIGN §1).
// Blocks without the comment marker, and agent-written blocks that are not
// comments, keep their bullets exactly as before.
import { openCommentThread } from "../tests/commentEntry";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { editingId, endEdit } from "../editorController";
import { blockRegions, initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { pageByName } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { contextMenu, setContextMenu } from "../ui";
import { dragId } from "./blockGestures";
import { shouldOpenBlockContextMenu } from "../contextMenuPolicy";
import { Block } from "./Block";

const PAGE = "MarginBulletless";

function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  const properties = blockRegions(raw, "md").properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, collapsed: false, children, ...(properties.length ? { properties } : {}) };
}

function mount(blocks: BlockDto[]): { root: HTMLElement; dispose: () => void } {
  const page: PageDto = { name: PAGE, kind: "page", title: PAGE, pre_block: null, blocks };
  loadSingle(page);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render((): JSX.Element => (
    <For each={pageByName(PAGE)?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  openCommentThread(root);
  return { root: document.body, dispose };
}

const fixture = () => mount([
  block("agent", "An agent wrote a passage worth disputing.\ntine.author:: claude", [
    block("c1", "I doubt this.\ntine.quote:: a passage worth disputing", [
      block("r1", "It holds, see the lemma.\ntine.author:: claude", [block("r2", "Fair enough.")]),
    ]),
    block("plain", "An ordinary child."),
  ]),
  block("other", "An unrelated block."),
]);

const row = (root: HTMLElement, id: string) => root.querySelector(`.ls-block[data-block-id="${id}"]`) as HTMLElement;
const ownControls = (el: HTMLElement) => el.querySelector(":scope > .block-main > .block-controls");

beforeAll(async () => {
  await initParser();
});
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  setContextMenu(null);
  document.body.innerHTML = "";
});

describe("comment blocks have no bullet", () => {
  it("draws no bullet or fold arrow on a comment or on any reply in its thread", () => {
    const { root, dispose } = fixture();
    try {
      for (const id of ["c1", "r1", "r2"]) {
        expect(ownControls(row(root, id)), id).toBeNull();
        expect(row(root, id).querySelector(":scope > .block-main .bullet-container"), id).toBeNull();
        // Still a rendered block with its text and its author chip as the handle.
        expect(row(root, id).querySelector(":scope > .comment-header .author-chip.comment-handle"), id).not.toBeNull();
      }
      expect(row(root, "r2").textContent).toContain("Fair enough.");
    } finally {
      dispose();
    }
  });

  it("keeps the bullet on blocks without the marker, an agent-written block included", () => {
    const { root, dispose } = fixture();
    try {
      for (const id of ["agent", "plain", "other"]) {
        expect(row(root, id).querySelector(":scope > .block-main > .block-controls > .bullet-container .bullet"), id).not.toBeNull();
      }
      // Only a comment's chip also offers its block menu.
      expect(row(root, "agent").querySelector(":scope > .block-main .author-chip")).not.toBeNull();
      expect(row(root, "agent").querySelector(":scope > .block-main .comment-handle")).toBeNull();
      expect(row(root, "plain").className).toBe("ls-block");
    } finally {
      dispose();
    }
  });
});

describe("the author chip is the comment's handle", () => {
  const chip = (root: HTMLElement, id: string) => row(root, id).querySelector(":scope > .comment-header .comment-handle") as HTMLElement;

  it.each(["c1", "r1"])("keeps the %s chip from starting a drag or entering the editor (GH #891)", (id) => {
    const { root, dispose } = fixture();
    try {
      const handle = chip(root, id);
      // A real pointer move must still leave this comment or reply in place.
      (document as { elementFromPoint: unknown }).elementFromPoint = () => null;
      handle.dispatchEvent(new MouseEvent("mousedown", { button: 0, clientX: 10, clientY: 10, bubbles: true, cancelable: true }));
      expect(editingId()).toBeNull();
      document.dispatchEvent(new MouseEvent("mousemove", { clientX: 40, clientY: 10, bubbles: true }));
      const moving = dragId();
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      expect(moving).toBeNull();
      delete (document as { elementFromPoint?: unknown }).elementFromPoint;
    } finally {
      dispose();
    }
  });

  it("opens the block context menu on right-click", () => {
    const { root, dispose } = fixture();
    try {
      chip(root, "c1").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }));
      expect(contextMenu()).toMatchObject({ kind: "block", blockId: "c1" });
    } finally {
      dispose();
    }
  });

  it("is a long-press menu target on touch platforms, where the bullet used to be the only one", () => {
    const { root, dispose } = fixture();
    try {
      expect(shouldOpenBlockContextMenu(chip(root, "c1"), true)).toBe(true);
      expect(shouldOpenBlockContextMenu(row(root, "agent").querySelector(".author-chip"), true)).toBe(false);
    } finally {
      dispose();
    }
  });
});

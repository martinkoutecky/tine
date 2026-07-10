import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { resetSheetRowVirtualizationForTests } from "./SheetTable";
import { resetBoardCardVirtualizationForTests } from "./SheetBoard";
import { initParser } from "../render/parse";
import { resetStore, setDoc, type FeedPage, type Node as StoreNode } from "../store";

// P2 lazy-mount virtualization guard. Two paths:
//  - EAGER: no IntersectionObserver (jsdom default) → observeNear fires
//    synchronously → every row/card renders real content, no placeholders.
//    This is the parity path every other sheet test runs on.
//  - DEFERRED: a no-op IntersectionObserver that never fires → every row/card
//    stays off-screen → heavy content is replaced by a `.sheet-cell-defer`
//    placeholder and the hover handle is NOT mounted.
// Necessity: delete the `when={near}`/`when={props.near}` gates and the DEFERRED
// assertions (placeholders present, handles absent) fail.

class NoopIO {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() { return []; }
  root = null;
  rootMargin = "";
  thresholds: number[] = [];
}

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetSheetRowVirtualizationForTests();
  resetBoardCardVirtualizationForTests();
  resetStore();
  document.body.innerHTML = "";
  delete (globalThis as any).IntersectionObserver;
});

function page(roots: string[]): FeedPage {
  return { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots, format: "md", readOnly: false, guide: false };
}
function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function tableDoc() {
  return {
    byId: {
      table: node("table", "Table\ntine.view:: table\ntine.fields:: owner=text;qty=number", null, ["r1", "r2"]),
      r1: node("r1", "TODO **Alpha** task\nowner:: Martin\nqty:: 3", "table"),
      r2: node("r2", "**Beta** task\nowner:: Jan\nqty:: 7", "table"),
    },
    pages: [page(["table"])],
    feed: ["Sheet"],
    loaded: true,
  };
}
function boardDoc() {
  return {
    byId: {
      board: node("board", "Board\ntine.view:: board\ntine.group-by:: prop:owner\ntine.fields:: owner=text", null, ["c1", "c2"]),
      c1: node("c1", "**Alpha** card\nowner:: Martin", "board"),
      c2: node("c2", "**Beta** card\nowner:: Jan", "board"),
    },
    pages: [page(["board"])],
    feed: ["Sheet"],
    loaded: true,
  };
}

describe("sheet lazy-mount virtualization", () => {
  it("table: eager path renders content, no placeholders; scaffolding always present", () => {
    setDoc(tableDoc());
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <Block id="table" />, root);
    // Every cell div is present (selection/nav/hit-test scaffolding).
    expect(root.querySelectorAll(".sheet-cell").length).toBeGreaterThan(0);
    // Eager: real content rendered (the TODO marker chip from InlineText), no defer spans.
    expect(root.querySelectorAll(".sheet-cell-defer").length).toBe(0);
    expect(root.querySelector(".block-marker")).not.toBeNull();
    dispose();
    root.remove();
  });

  it("table: deferred path keeps every cell div but replaces content with placeholders and drops handles", () => {
    (globalThis as any).IntersectionObserver = NoopIO;
    setDoc(tableDoc());
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <Block id="table" />, root);
    // Scaffolding unchanged: all cell divs still there.
    expect(root.querySelectorAll(".sheet-cell").length).toBeGreaterThan(0);
    // Content deferred: placeholders present, no parsed marker, no hover handle.
    expect(root.querySelectorAll(".sheet-cell-defer").length).toBeGreaterThan(0);
    expect(root.querySelector(".block-marker")).toBeNull();
    expect(root.querySelector(".sheet-cell-handle")).toBeNull();
    // data-row/col locators (drag + keyboard nav depend on them) survive deferral.
    expect(root.querySelector('.sheet-cell[data-row="0"][data-col="0"]')).not.toBeNull();
    dispose();
    root.remove();
  });

  it("board: deferred path defers card content but keeps the card shell + locators", () => {
    (globalThis as any).IntersectionObserver = NoopIO;
    setDoc(boardDoc());
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <Block id="board" />, root);
    expect(root.querySelectorAll(".sheet-board-card").length).toBeGreaterThan(0);
    expect(root.querySelectorAll(".sheet-cell-defer").length).toBeGreaterThan(0);
    expect(root.querySelector(".sheet-card-handle")).toBeNull();
    // A card keeps its data-block-id so drag hit-testing / selection resolve.
    expect(root.querySelector(".sheet-board-card[data-block-id]")).not.toBeNull();
    dispose();
    root.remove();
  });

  it("board: eager path renders card titles and chips, no placeholders", () => {
    setDoc(boardDoc());
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <Block id="board" />, root);
    expect(root.querySelectorAll(".sheet-board-card").length).toBeGreaterThan(0);
    expect(root.querySelectorAll(".sheet-cell-defer").length).toBe(0);
    expect(root.querySelector(".sheet-board-card-title")).not.toBeNull();
    dispose();
    root.remove();
  });
});

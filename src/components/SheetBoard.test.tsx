import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { doc, resetStore, setDoc, type FeedPage, type Node as StoreNode } from "../store";
import { setWorkflow } from "../ui";
import { cellSel, handleCellSelectionKey, resetCellSelectionForTests, setCellSel } from "../sheet/selection";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetCellSelectionForTests();
  resetStore();
  setWorkflow("todo");
  document.body.innerHTML = "";
});

beforeEach(() => {
  setWorkflow("todo");
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function page(roots: string[]): FeedPage {
  return {
    name: "Sheet",
    kind: "page",
    title: "Sheet",
    preBlock: null,
    roots,
    format: "md",
    readOnly: false,
  };
}

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function loadBoardDoc(todoRaw = "TODO Write tests\nowner:: Codex") {
  setDoc({
    byId: {
      board: node("board", "Board\ntine.view:: board\ntine.group-by:: state", null, ["todo", "doing", "plain"]),
      todo: node("todo", todoRaw, "board"),
      doing: node("doing", "DOING Implement board", "board"),
      plain: node("plain", "No marker", "board"),
    },
    pages: [page(["board"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

function keydown(key: string, init: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return new KeyboardEvent("keydown", {
    key,
    code: key.startsWith("Arrow") ? key : "",
    bubbles: true,
    cancelable: true,
    ctrlKey: init.ctrlKey ?? false,
    metaKey: init.metaKey ?? false,
    shiftKey: init.shiftKey ?? false,
    altKey: init.altKey ?? false,
  });
}

function pointer(type: string, x: number, y: number): Event {
  return new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: x, clientY: y });
}

describe("SheetBoard", () => {
  it("groups cards by state with counts and a trailing none column", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <Block id="board" />);

    const headers = [...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim());
    expect(headers).toEqual(["TODO1", "DOING1", "DONE0", "(none)1"]);

    dispose();
  });

  it("orders prop enum group columns by the declared schema before extras and none", () => {
    setDoc({
      byId: {
        board: node(
          "board",
          "Board\ntine.view:: board\ntine.group-by:: prop:status\ntine.fields:: status=enum:todo,doing,done",
          null,
          ["done", "blocked", "plain"]
        ),
        done: node("done", "Finished\nstatus:: done", "board"),
        blocked: node("blocked", "Blocked\nstatus:: blocked", "board"),
        plain: node("plain", "No status", "board"),
      },
      pages: [page(["board"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="board" />);

    const headers = [...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim());
    expect(headers).toEqual(["todo0", "doing0", "done1", "blocked1", "(none)1"]);

    dispose();
  });

  it("renders a block highlight color on the card", () => {
    loadBoardDoc("TODO Write tests\nbackground-color:: blue\nowner:: Codex");
    const { root, dispose } = mount(() => <Block id="board" />);

    const card = root.querySelector('[data-block-id="todo"]') as HTMLElement | null;
    expect(card).not.toBeNull();
    expect(card!.style.background).toContain("rgba");

    dispose();
  });

  it("Ctrl+Arrow moves a selected card to the adjacent state column", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <Block id="board" />);

    setCellSel({ gridId: "board", row: 0, col: 0 });
    const event = keydown("ArrowRight", { ctrlKey: true });

    expect(handleCellSelectionKey(event)).toBe(true);
    expect(doc.byId.todo.raw.split("\n")[0]).toBe("DOING Write tests");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "board", row: 0, col: 1 });
    expect([...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim())).toEqual([
      "TODO0",
      "DOING2",
      "DONE0",
      "(none)1",
    ]);

    dispose();
  });

  it("dragging a card to another column writes only that card marker", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <Block id="board" />);
    const card = root.querySelector('[data-block-id="todo"]') as HTMLElement;
    const target = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => target;

    card.dispatchEvent(pointer("pointerdown", 0, 0));
    document.dispatchEvent(pointer("pointermove", 12, 0));
    document.dispatchEvent(pointer("pointerup", 12, 0));

    expect(doc.byId.todo.raw.split("\n")[0]).toBe("DOING Write tests");
    expect(doc.byId.doing.raw.split("\n")[0]).toBe("DOING Implement board");

    document.elementFromPoint = prevElementFromPoint;
    dispose();
  });
});

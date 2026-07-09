import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal, type JSX } from "solid-js";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { SheetBoard } from "./SheetBoard";
import { initParser } from "../render/parse";
import { blockProperty, doc, hasSelection, resetStore, setDoc, undo, type FeedPage, type Node as StoreNode } from "../store";
import { closeContextMenu, openSheetContextMenu, setToasts, setWorkflow, toasts } from "../ui";
import { cellSel, handleCellSelectionKey, resetCellSelectionForTests, setCellSel } from "../sheet/selection";
import { installBlockSelectionDrag } from "../blockDrag";
import type { RefGroup } from "../types";
import { backend } from "../backend";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  closeContextMenu();
  resetCellSelectionForTests();
  resetStore();
  setToasts([]);
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
    guide: false,
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

function loadTagBoard(rows: Record<string, string>) {
  const ids = Object.keys(rows);
  setDoc({
    byId: {
      board: node("board", "Board\ntine.view:: board\ntine.group-by:: tags", null, ids),
      ...Object.fromEntries(ids.map((id) => [id, node(id, rows[id], "board")])),
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

function contextMenu(target: EventTarget): MouseEvent {
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 30 });
  target.dispatchEvent(event);
  return event;
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function queryGroups(ids: string[]): RefGroup[] {
  return [{
    page: "Sheet",
    kind: "page",
    blocks: ids.map((id) => ({
      id,
      raw: doc.byId[id].raw,
      collapsed: false,
      children: [],
    })),
  }];
}

describe("SheetBoard", () => {
  it("changes group-by from the board context menu submenu and undo restores it", async () => {
    setDoc({
      byId: {
        board: node("board", "Board\ntine.view:: board\ntine.group-by:: state", null, ["todo", "doing", "plain"]),
        todo: node("todo", "TODO Write tests #alpha\nowner:: Codex", "board"),
        doing: node("doing", "DOING Implement board #beta", "board"),
        plain: node("plain", "No marker", "board"),
      },
      pages: [page(["board"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => (
      <>
        <Block id="board" />
        <ContextMenu />
      </>
    ));

    const board = root.querySelector(".sheet-board") as HTMLElement | null;
    expect(board).not.toBeNull();
    contextMenu(board!);

    const submenu = [...document.querySelectorAll(".ctx-submenu")].find((el) =>
      el.textContent?.includes("Group by")
    ) as HTMLElement | undefined;
    expect(submenu).toBeTruthy();
    const items = [...submenu!.querySelectorAll(".ctx-submenu-menu > .ctx-item")] as HTMLElement[];
    expect(items.map((el) => el.textContent?.trim())).toEqual(["✓ State", "Priority", "Tags", "owner"]);
    expect(items[0].classList.contains("ctx-active")).toBe(true);

    items.find((el) => el.textContent?.trim() === "Tags")!.click();
    await tick();

    expect(blockProperty("board", "tine.group-by")).toBe("tags");
    expect([...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim())).toEqual([
      "alpha1",
      "beta1",
      "(none)1",
    ]);

    undo();
    await tick();

    expect(blockProperty("board", "tine.group-by")).toBe("state");
    expect([...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim())).toEqual([
      "TODO1",
      "DOING1",
      "DONE0",
      "(none)1",
    ]);

    dispose();
  });

  it("header group-by select writes the property and regroups", async () => {
    setDoc({
      byId: {
        board: node("board", "Board\ntine.view:: board\ntine.group-by:: state", null, ["a", "b", "plain"]),
        a: node("a", "TODO [#A] Write tests", "board"),
        b: node("b", "DOING [#B] Implement board", "board"),
        plain: node("plain", "No priority", "board"),
      },
      pages: [page(["board"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="board" />);
    const select = root.querySelector(".sheet-board-groupby") as HTMLSelectElement | null;
    expect(select).not.toBeNull();
    expect(select!.value).toBe("state");

    select!.value = "priority";
    select!.dispatchEvent(new Event("change", { bubbles: true }));
    await tick();

    expect(blockProperty("board", "tine.group-by")).toBe("priority");
    expect([...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim())).toEqual([
      "[#A]1",
      "[#B]1",
      "[#C]0",
      "(none)1",
    ]);

    dispose();
  });

  it("group-by submenu is gated to board sheet context menus", async () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <ContextMenu />);

    openSheetContextMenu(20, 30, "board", "grid", "children");
    await tick();
    expect(root.textContent).not.toContain("Group by");

    openSheetContextMenu(20, 30, "board", "table", "children");
    await tick();
    expect(root.textContent).not.toContain("Group by");
    expect(root.querySelector(".sheet-board-groupby")).toBeNull();

    dispose();
  });

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

  it("groups by formula values and refuses Ctrl+Arrow moves on the derived axis", () => {
    setDoc({
      byId: {
        board: node(
          "board",
          "Board\ntine.view:: board\ntine.group-by:: formula.big\ntine.formula.big:: points > 2",
          null,
          ["high", "low"]
        ),
        high: node("high", "High\npoints:: 3", "board"),
        low: node("low", "Low\npoints:: 1", "board"),
      },
      pages: [page(["board"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="board" />);
    const before = doc.byId.high.raw;

    expect([...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim())).toEqual([
      "true1",
      "false1",
    ]);
    setCellSel({ gridId: "board", row: 0, col: 0 });
    expect(handleCellSelectionKey(keydown("ArrowRight", { ctrlKey: true }))).toBe(true);
    expect(doc.byId.high.raw).toBe(before);

    dispose();
  });

  it("applies board filters before grouping and shows all cards when a filter is broken", () => {
    setDoc({
      byId: {
        board: node(
          "board",
          "Board\ntine.view:: board\ntine.group-by:: state\ntine.filter:: points > 2",
          null,
          ["high", "low"]
        ),
        high: node("high", "TODO High\npoints:: 3", "board"),
        low: node("low", "DOING Low\npoints:: 1", "board"),
      },
      pages: [page(["board"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="board" />);

    expect([...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim())).toEqual([
      "TODO1",
      "DOING0",
      "DONE0",
    ]);
    dispose();

    setDoc("byId", "board", "raw", "Board\ntine.view:: board\ntine.group-by:: state\ntine.filter:: points >");
    const mounted = mount(() => <Block id="board" />);
    expect([...mounted.root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim())).toEqual([
      "TODO1",
      "DOING1",
      "DONE0",
    ]);
    const chip = mounted.root.querySelector(".sheet-filter-error") as HTMLElement | null;
    expect(chip?.getAttribute("title")).toContain("Filter parse error");

    mounted.dispose();
  });

  it("dragging a card to another column writes only that card marker", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <Block id="board" />);
    const card = root.querySelector('[data-block-id="todo"]') as HTMLElement;
    const target = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => target;

    card.dispatchEvent(pointer("pointerdown", 0, 0));
    window.dispatchEvent(pointer("pointermove", 12, 0));
    window.dispatchEvent(pointer("pointerup", 12, 0));

    expect(doc.byId.todo.raw.split("\n")[0]).toBe("DOING Write tests");
    expect(doc.byId.doing.raw.split("\n")[0]).toBe("DOING Implement board");

    document.elementFromPoint = prevElementFromPoint;
    dispose();
  });

  it("shows a floating drag ghost with grabbing cursor state and removes it on Escape", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <Block id="board" />);
    const card = root.querySelector('[data-block-id="todo"]') as HTMLElement;
    const target = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => target;

    card.dispatchEvent(pointer("pointerdown", 0, 0));
    window.dispatchEvent(pointer("pointermove", 12, 0));

    const ghost = document.body.querySelector(".sheet-board-drag-ghost") as HTMLElement | null;
    expect(ghost).not.toBeNull();
    expect(ghost!.style.transform).toContain("translate(22px, 10px)");
    expect(document.body.classList.contains("sheet-board-dragging")).toBe(true);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));

    expect(document.body.querySelector(".sheet-board-drag-ghost")).toBeNull();
    expect(document.body.classList.contains("sheet-board-dragging")).toBe(false);
    expect(doc.byId.todo.raw.split("\n")[0]).toBe("TODO Write tests");

    document.elementFromPoint = prevElementFromPoint;
    dispose();
  });

  it("does not start outline multi-block selection from a board card drag origin", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <Block id="board" />);
    const uninstall = installBlockSelectionDrag();
    const card = root.querySelector('[data-block-id="todo"]') as HTMLElement;
    const over = document.createElement("div");
    over.className = "ls-block";
    over.dataset.blockId = "doing";
    document.body.appendChild(over);
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => over;

    try {
      card.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, cancelable: true, buttons: 1, clientX: 12, clientY: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));

      expect(hasSelection()).toBe(false);
    } finally {
      document.elementFromPoint = prevElementFromPoint;
      uninstall();
      dispose();
    }
  });

  it("renders tag boards with duplicated multi-tag cards and a none column", () => {
    loadTagBoard({
      multi: "Read #alpha #beta",
      empty: "No tags",
      beta: "Beta only #beta",
    });
    const { root, dispose } = mount(() => <Block id="board" />);

    const headers = [...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim());
    expect(headers).toEqual(["alpha1", "beta2", "(none)1"]);
    expect(root.querySelectorAll('[data-block-id="multi"]')).toHaveLength(2);
    expect(root.querySelector('[data-board-col="2"] [data-block-id="empty"]')).not.toBeNull();

    dispose();
  });

  it("Ctrl+Arrow on a tag board rewrites the raw tag token", () => {
    loadTagBoard({
      keep: "Keep #alpha",
      move: "Read #alpha",
      beta: "Other #beta",
    });
    const { dispose } = mount(() => <Block id="board" />);

    setCellSel({ gridId: "board", row: 1, col: 0 });
    expect(handleCellSelectionKey(keydown("ArrowRight", { ctrlKey: true }))).toBe(true);

    expect(doc.byId.move.raw).toBe("Read #beta");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "board", row: 0, col: 1 });

    dispose();
  });

  it("dragging a tag card between columns removes the source tag and appends the target tag", () => {
    loadTagBoard({
      move: "Read #alpha",
      beta: "Other #beta",
    });
    const { root, dispose } = mount(() => <Block id="board" />);
    const card = root.querySelector('[data-block-id="move"]') as HTMLElement;
    const target = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => target;

    card.dispatchEvent(pointer("pointerdown", 0, 0));
    window.dispatchEvent(pointer("pointermove", 12, 0));
    window.dispatchEvent(pointer("pointerup", 12, 0));

    expect(doc.byId.move.raw).toBe("Read #beta");

    document.elementFromPoint = prevElementFromPoint;
    dispose();
  });

  it("adds a session-only tag column and persists it when a card is moved there", async () => {
    loadTagBoard({
      move: "Read #alpha",
    });
    const { root, dispose } = mount(() => <Block id="board" />);

    (root.querySelector(".sheet-board-add-tag-ghost") as HTMLButtonElement).click();
    let input = root.querySelector(".sheet-board-add-tag-input") as HTMLInputElement;
    input.value = "bad tag";
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    expect(root.querySelectorAll(".sheet-board-header")).toHaveLength(1);
    expect(input.classList.contains("sheet-input-invalid")).toBe(true);

    input.value = "gamma";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await tick();

    expect([...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim())).toEqual([
      "alpha1",
      "gamma0",
    ]);

    const card = root.querySelector('[data-block-id="move"]') as HTMLElement;
    const target = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => target;
    card.dispatchEvent(pointer("pointerdown", 0, 0));
    window.dispatchEvent(pointer("pointermove", 12, 0));
    window.dispatchEvent(pointer("pointerup", 12, 0));
    document.elementFromPoint = prevElementFromPoint;
    await tick();

    expect(doc.byId.move.raw).toBe("Read #gamma");
    expect([...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim())).toEqual(["gamma1"]);

    dispose();
  });

  it("toasts once when a query-board move leaves the refreshed query rows", async () => {
    loadBoardDoc();
    let setGroups!: (groups: RefGroup[]) => void;
    const Harness = () => {
      const [groups, writeGroups] = createSignal<RefGroup[]>(queryGroups(["todo"]));
      setGroups = writeGroups;
      return <SheetBoard ownerId="board" rowSource="query" groupBy="state" groups={groups()} />;
    };
    const { dispose } = mount(() => <Harness />);

    setCellSel({ gridId: "board", row: 0, col: 0 });
    expect(handleCellSelectionKey(keydown("ArrowRight", { ctrlKey: true }))).toBe(true);
    expect(toasts()).toHaveLength(0);

    setGroups(queryGroups([]));
    await tick();

    expect(toasts().map((t) => [t.message, t.kind])).toEqual([
      ["Moved out of this query's results", "info"],
    ]);

    setGroups(queryGroups([]));
    await tick();
    expect(toasts()).toHaveLength(1);

    dispose();
  });

  it("does not toast when a query-board move remains in refreshed query rows", async () => {
    loadBoardDoc();
    let setGroups!: (groups: RefGroup[]) => void;
    const Harness = () => {
      const [groups, writeGroups] = createSignal<RefGroup[]>(queryGroups(["todo"]));
      setGroups = writeGroups;
      return <SheetBoard ownerId="board" rowSource="query" groupBy="state" groups={groups()} />;
    };
    const { dispose } = mount(() => <Harness />);

    setCellSel({ gridId: "board", row: 0, col: 0 });
    expect(handleCellSelectionKey(keydown("ArrowRight", { ctrlKey: true }))).toBe(true);

    setGroups(queryGroups(["todo"]));
    await tick();

    expect(toasts()).toHaveLength(0);

    dispose();
  });

  it("refuses to move a multi-tag card into none", () => {
    loadTagBoard({
      multi: "Read #alpha #beta",
      empty: "No tags",
    });
    const { dispose } = mount(() => <Block id="board" />);

    setCellSel({ gridId: "board", row: 0, col: 1 });
    expect(handleCellSelectionKey(keydown("ArrowRight", { ctrlKey: true }))).toBe(true);
    expect(doc.byId.multi.raw).toBe("Read #alpha #beta");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "board", row: 0, col: 1 });

    dispose();
  });

  it("selects only one coordinate for a duplicated tag card", () => {
    loadTagBoard({ multi: "Read #alpha #beta" });
    const { root, dispose } = mount(() => <Block id="board" />);

    setCellSel({ gridId: "board", row: 0, col: 1 });

    const cards = [...root.querySelectorAll('[data-block-id="multi"]')] as HTMLElement[];
    expect(cards).toHaveLength(2);
    expect(cards.map((card) => card.classList.contains("sheet-cell-selected"))).toEqual([false, true]);

    dispose();
  });

  it("renders a query board exactly once when the query block has board view props", async () => {
    setDoc({
      byId: {
        query: node("query", "{{query (todo TODO)}}\ntine.view:: board\ntine.group-by:: state", null),
        todo: node("todo", "TODO From query", null),
      },
      pages: [page(["query", "todo"])],
      feed: ["Sheet"],
      loaded: true,
    });
    vi.spyOn(backend(), "runQuery").mockResolvedValue(queryGroups(["todo"]));

    const { root, dispose } = mount(() => <Block id="query" />);
    await tick();
    await tick();

    expect(root.querySelectorAll(".sheet-board")).toHaveLength(1);
    expect(root.querySelector(".block-sheet-container > .sheet-scroll > .sheet-board-wrap > .sheet-board")).not.toBeNull();
    expect(root.textContent).toContain("From query");

    dispose();
  });

  it("renders exactly one board when the query macro shares its block with a heading (Martin's ghost board)", async () => {
    // The §4 demo regression: heading + {{query}} + view props in ONE block, no
    // children. detectMacro (exact-body) misses this shape, so the
    // children-source face used to render a second, empty workflow board.
    setDoc({
      byId: {
        query: node(
          "query",
          "## 4 · Task kanban\n{{query (todo TODO)}}\ntine.view:: board\ntine.group-by:: state",
          null
        ),
        todo: node("todo", "TODO From query", null),
      },
      pages: [page(["query", "todo"])],
      feed: ["Sheet"],
      loaded: true,
    });
    vi.spyOn(backend(), "runQuery").mockResolvedValue(queryGroups(["todo"]));

    const { root, dispose } = mount(() => <Block id="query" />);
    await tick();
    await tick();

    expect(root.querySelectorAll(".sheet-board")).toHaveLength(1);
    expect(root.textContent).toContain("From query");

    dispose();
  });
});

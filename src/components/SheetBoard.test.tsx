import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal, type JSX } from "solid-js";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { __sheetBoardTestHooks, SheetBoard } from "./SheetBoard";
import { initParser } from "../render/parse";
import { blockProperty, doc, hasSelection, resetStore, setDoc, setRaw, undo, type FeedPage, type Node as StoreNode } from "../store";
import { closeContextMenu, openSheetContextMenu, setToasts, setWorkflow, toasts } from "../ui";
import { cellForBlockId, cellOwner, cellSel, handleCellSelectionKey, resetCellSelectionForTests, setCellSel, startCellEditing } from "../sheet/selection";
import { installBlockSelectionDrag } from "../blockDrag";
import type { RefGroup } from "../types";
import { backend } from "../backend";
import { installKeybindings } from "../keybindings";
import { clearTransientLayersForTest, registerTransientLayer } from "../transientLayers";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  clearTransientLayersForTest();
  __sheetBoardTestHooks.onGroupingRowWalk = undefined;
  __sheetBoardTestHooks.onPointIndexRow = undefined;
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

function pointer(type: string, x: number, y: number, pointerId = 1): Event {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: x, clientY: y });
  Object.defineProperty(event, "pointerId", { value: pointerId });
  return event;
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

function boardColumnBlockIds(root: ParentNode, col: number): string[] {
  return [...root.querySelectorAll(`[data-board-col="${col}"] [data-block-id]`)]
    .map((card) => (card as HTMLElement).dataset.blockId ?? "");
}

describe("SheetBoard", () => {
  it("keeps a same-UUID journal twin DTO-only when the page twin is preloaded", async () => {
    const shared = "same-id";
    setDoc({
      byId: {
        board: node("board", "Board\ntine.view:: board\ntine.group-by:: state", null),
        [shared]: { ...node(shared, "TODO Loaded page row", null), page: "Twin" },
      },
      pages: [
        page(["board"]),
        { ...page([shared]), name: "Twin", title: "Twin", kind: "page" },
      ],
      feed: ["Sheet"], loaded: true,
    });
    const groups: RefGroup[] = [
      { page: "Twin", kind: "page", blocks: [{ id: shared, raw: "TODO Page DTO", collapsed: false, children: [] }] },
      { page: "Twin", kind: "journal", blocks: [{ id: shared, raw: "DOING Journal DTO", marker: "DOING", collapsed: false, children: [] }] },
    ];
    const { root, dispose } = mount(() => (
      <SheetBoard ownerId="board" rowSource="query" groupBy="state" groups={groups} />
    ));
    expect(root.textContent).toContain("Loaded page row");
    expect(root.textContent).toContain("Journal DTO");
    const doingColumn = [...root.querySelectorAll<HTMLElement>(".sheet-board-column")].find(
      (column) => column.querySelector(".sheet-board-header span")?.textContent === "DOING"
    )!;
    expect([...doingColumn.querySelectorAll<HTMLElement>(".sheet-board-card")].map((card) => card.textContent?.trim()))
      .toEqual(["Journal DTO"]);
    const journalCard = doingColumn.querySelector<HTMLElement>(`[data-block-id="${shared}"]`)!;

    expect(startCellEditing({ gridId: "board", surfaceId: "main", row: 0, col: 1 })).toBe(false);

    journalCard.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 }));
    await tick();
    expect(root.querySelector("textarea.block-editor")).toBeNull();
    journalCard.click();
    expect(handleCellSelectionKey(keydown("ArrowRight", { ctrlKey: true }))).toBe(true);
    expect(doc.byId[shared].raw).toBe("TODO Loaded page row");
    dispose();
  });

  it("paginates same-UUID page and journal twins by composite row identity", () => {
    const shared = "same-id";
    const filler = Array.from({ length: 199 }, (_, i) => ({
      id: `f${i}`,
      raw: `TODO Filler ${i}`,
      collapsed: false,
      children: [],
    }));
    setDoc({
      byId: {
        board: node("board", "Board\ntine.view:: board\ntine.group-by:: state", null),
        [shared]: { ...node(shared, "TODO Loaded page row", null), page: "Twin" },
      },
      pages: [page(["board"]), { ...page([shared]), name: "Twin", title: "Twin", kind: "page" }],
      feed: ["Sheet"],
      loaded: true,
    });
    const groups: RefGroup[] = [
      {
        page: "Twin",
        kind: "page",
        blocks: [{ id: shared, raw: "TODO Page DTO", collapsed: false, children: [] }, ...filler],
      },
      {
        page: "Twin",
        kind: "journal",
        blocks: [{ id: shared, raw: "DOING Journal DTO", marker: "DOING", collapsed: false, children: [] }],
      },
    ];
    const { root, dispose } = mount(() => (
      <SheetBoard ownerId="board" rowSource="query" groupBy="state" groups={groups} />
    ));

    expect(root.textContent).not.toContain("Journal DTO");
    (root.querySelector(".sheet-load-more") as HTMLButtonElement).click();
    expect(root.textContent).toContain("Journal DTO");
    dispose();
  });

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

  it("clears a stable card selection when a query refresh removes that row", async () => {
    loadBoardDoc();
    const [groups, setGroups] = createSignal<RefGroup[]>(queryGroups(["todo", "doing"]));
    const { dispose } = mount(() => (
      <SheetBoard ownerId="board" rowSource="query" groupBy="state" groups={groups()} />
    ));
    setCellSel({ gridId: "board", surfaceId: "main", rowId: "page\0Sheet\0todo", row: 0, col: 0 });
    const selected = cellSel();
    expect(selected?.kind === "cell" ? selected.rowId : null).toBe("page\0Sheet\0todo");

    setGroups(queryGroups(["doing"]));
    await tick();

    expect(cellSel()).toBeNull();
    const before = doc.byId.doing.raw;
    expect(handleCellSelectionKey(keydown("ArrowRight", { ctrlKey: true }))).toBe(false);
    expect(doc.byId.doing.raw).toBe(before);
    dispose();
  });

  it("rebases a selected card by row identity before the next keyboard move", async () => {
    loadBoardDoc();
    const { dispose } = mount(() => <Block id="board" />);
    setCellSel({ gridId: "board", surfaceId: "main", rowId: "todo", row: 0, col: 0 });

    setRaw("todo", "DOING Write tests\nowner:: Codex", { timetracking: false });
    await tick();

    const selected = cellSel();
    expect(selected?.kind === "cell" ? { row: selected.row, col: selected.col, rowId: selected.rowId } : null)
      .toEqual({ row: 0, col: 1, rowId: "todo" });
    const otherBefore = doc.byId.doing.raw;
    expect(handleCellSelectionKey(keydown("ArrowRight", { ctrlKey: true }))).toBe(true);
    expect(doc.byId.todo.raw.split("\n")[0]).toBe("DONE Write tests");
    expect(doc.byId.doing.raw).toBe(otherBefore);
    dispose();
  });

  it("uses the cached membership index for repeated navigation in a large query board", () => {
    setDoc({
      byId: { board: node("board", "Board\ntine.view:: board\ntine.group-by:: state", null) },
      pages: [page(["board"])], feed: ["Sheet"], loaded: true,
    });
    const blocks = Array.from({ length: 5_000 }, (_, i) => ({
      id: `q${i}`,
      raw: `TODO Row ${i}`,
      collapsed: false,
      children: [],
    }));
    let indexed = 0;
    __sheetBoardTestHooks.onPointIndexRow = () => indexed++;
    const { root, dispose } = mount(() => (
      <SheetBoard ownerId="board" rowSource="query" groupBy="state" groups={[{ page: "Remote", kind: "page", blocks }]} />
    ));
    (root.querySelector('[data-block-id="q0"]') as HTMLElement).click();
    expect(indexed).toBe(5_000);
    for (let i = 0; i < 20; i++) handleCellSelectionKey(keydown("ArrowDown"));
    expect(indexed).toBe(5_000);
    dispose();
  });

  it("extends the query window before a refreshed off-window card can move", async () => {
    const target = "live-target";
    const filler = Array.from({ length: 270 }, (_, i) => ({
      id: `f${i}`, raw: `TODO Filler ${i}`, marker: "TODO", collapsed: false, children: [],
    }));
    const live = { id: target, raw: "TODO Target", marker: "TODO", collapsed: false, children: [] };
    const remote: FeedPage = {
      name: "Remote", kind: "page", title: "Remote", preBlock: null, roots: [target],
      format: "md", readOnly: false, guide: false,
    };
    setDoc({
      byId: {
        board: node("board", "Board\ntine.view:: board\ntine.group-by:: state", null),
        [target]: { ...node(target, live.raw, null), page: remote.name },
      },
      pages: [page(["board"]), remote], feed: ["Sheet"], loaded: true,
    });
    const group = (blocks: typeof filler): RefGroup[] => [{ page: remote.name, kind: "page", blocks }];
    const [groups, setGroups] = createSignal(group([live, ...filler]));
    const { root, dispose } = mount(() => (
      <SheetBoard ownerId="board" rowSource="query" groupBy="state" groups={groups()} />
    ));
    setCellSel({ gridId: "board", surfaceId: "main", rowId: `page\0Remote\0${target}`, columnId: JSON.stringify("TODO"), row: 0, col: 0 });

    setGroups(group([...filler.slice(0, 260), live, ...filler.slice(260)]));

    const selected = cellSel();
    expect(selected?.kind === "cell" ? selected.row : -1).toBe(260);
    expect(root.querySelector(`[data-block-id="${target}"]`)).not.toBeNull();
    expect(handleCellSelectionKey(keydown("ArrowRight", { ctrlKey: true }))).toBe(true);
    expect(doc.byId[target].raw.startsWith("DOING Target")).toBe(true);
    expect(root.querySelector(`[data-board-col="1"] [data-block-id="${target}"]`)).not.toBeNull();
    dispose();
  });

  it("clears a stale coordinate-only selection after query pagination resets instead of moving a hidden card", () => {
    const target = "live-target";
    const filler = Array.from({ length: 270 }, (_, i) => ({
      id: `f${i}`, raw: `TODO Filler ${i}`, marker: "TODO", collapsed: false, children: [],
    }));
    const live = { id: target, raw: "TODO Hidden target", marker: "TODO", collapsed: false, children: [] };
    const remote: FeedPage = {
      name: "Remote", kind: "page", title: "Remote", preBlock: null, roots: [target],
      format: "md", readOnly: false, guide: false,
    };
    setDoc({
      byId: {
        board: node("board", "Board\ntine.view:: board\ntine.group-by:: state", null),
        [target]: { ...node(target, live.raw, null), page: remote.name },
      },
      pages: [page(["board"]), remote], feed: ["Sheet"], loaded: true,
    });
    const makeGroups = (): RefGroup[] => [{ page: remote.name, kind: "page", blocks: [...filler.slice(0, 260), live, ...filler.slice(260)] }];
    const [groups, setGroups] = createSignal(makeGroups());
    const { root, dispose } = mount(() => (
      <SheetBoard ownerId="board" rowSource="query" groupBy="state" groups={groups()} />
    ));
    (root.querySelector(".sheet-load-more") as HTMLButtonElement).click();
    expect(root.querySelector(`[data-block-id="${target}"]`)).not.toBeNull();
    setCellSel({ gridId: "board", surfaceId: "main", row: 260, col: 0 });

    setGroups(makeGroups());
    expect(root.querySelector(`[data-block-id="${target}"]`)).toBeNull();
    const before = doc.byId[target].raw;
    expect(handleCellSelectionKey(keydown("ArrowRight", { ctrlKey: true }))).toBe(true);

    expect(doc.byId[target].raw).toBe(before);
    expect(cellSel()).toBeNull();
    dispose();
  });

  it("uses the 100k membership index for repeated cellForBlock nested-ascent lookups without rescanning", () => {
    const target = "live-target";
    const blocks = Array.from({ length: 100_000 }, (_, i) => ({
      id: i === 0 ? target : `q${i}`,
      raw: `TODO Row ${i}`,
      marker: "TODO",
      collapsed: false,
      children: [],
    }));
    const remote: FeedPage = {
      name: "Remote", kind: "page", title: "Remote", preBlock: null, roots: [target],
      format: "md", readOnly: false, guide: false,
    };
    setDoc({
      byId: {
        board: node("board", "Board\ntine.view:: board\ntine.group-by:: state", null),
        [target]: { ...node(target, "TODO Row 0", null), page: remote.name },
      },
      pages: [page(["board"]), remote], feed: ["Sheet"], loaded: true,
    });
    let indexed = 0;
    __sheetBoardTestHooks.onPointIndexRow = () => indexed++;
    const { dispose } = mount(() => (
      <SheetBoard ownerId="board" rowSource="query" groupBy="state" groups={[{ page: remote.name, kind: "page", blocks }]} />
    ));
    expect(indexed).toBe(100_000);
    for (let i = 0; i < 50; i++) {
      expect(cellForBlockId(target, "main")?.rowId).toBe(`page\0Remote\0${target}`);
    }
    expect(indexed).toBe(100_000);
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

  it("keeps only the latest pointer drag session active across Board instances", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => (
      <>
        <SheetBoard ownerId="board" rowSource="children" groupBy="state" />
        <SheetBoard ownerId="board" rowSource="children" groupBy="state" />
      </>
    ));
    const boards = root.querySelectorAll(".sheet-board");
    const todo = boards[0].querySelector('[data-block-id="todo"]') as HTMLElement;
    const doing = boards[1].querySelector('[data-block-id="doing"]') as HTMLElement;
    const doingColumn = boards[0].querySelector('[data-board-col="1"]') as HTMLElement;
    const doneColumn = boards[1].querySelector('[data-board-col="2"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = (x) => x >= 200 ? doneColumn : doingColumn;

    try {
      todo.dispatchEvent(pointer("pointerdown", 0, 0, 11));
      window.dispatchEvent(pointer("pointermove", 12, 0, 11));
      expect(document.body.querySelectorAll(".sheet-board-drag-ghost")).toHaveLength(1);

      doing.dispatchEvent(pointer("pointerdown", 100, 0, 22));
      expect(document.body.querySelectorAll(".sheet-board-drag-ghost")).toHaveLength(0);

      window.dispatchEvent(pointer("pointerup", 12, 0, 11));
      expect(doc.byId.todo.raw.split("\n")[0]).toBe("TODO Write tests");
      expect(doc.byId.doing.raw.split("\n")[0]).toBe("DOING Implement board");

      window.dispatchEvent(pointer("pointermove", 212, 0, 22));
      window.dispatchEvent(pointer("pointerup", 212, 0, 22));
      expect(doc.byId.todo.raw.split("\n")[0]).toBe("TODO Write tests");
      expect(doc.byId.doing.raw.split("\n")[0]).toBe("DONE Implement board");
    } finally {
      document.elementFromPoint = prevElementFromPoint;
      dispose();
    }
  });

  it("rejects a drop column belonging to a duplicate Board surface", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => (
      <>
        <SheetBoard ownerId="board" rowSource="children" groupBy="state" />
        <SheetBoard ownerId="board" rowSource="children" groupBy="state" />
      </>
    ));
    const boards = root.querySelectorAll(".sheet-board");
    const sourceCard = boards[0].querySelector('[data-block-id="todo"]') as HTMLElement;
    const foreignTarget = boards[1].querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => foreignTarget;

    try {
      sourceCard.dispatchEvent(pointer("pointerdown", 0, 0, 31));
      window.dispatchEvent(pointer("pointermove", 12, 0, 31));
      window.dispatchEvent(pointer("pointerup", 12, 0, 31));

      expect(doc.byId.todo.raw.split("\n")[0]).toBe("TODO Write tests");
      expect(foreignTarget.classList.contains("sheet-board-drop")).toBe(false);
    } finally {
      document.elementFromPoint = prevElementFromPoint;
      dispose();
    }
  });

  it("re-hit-tests the pointerup position instead of dropping on a stale move target", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <Block id="board" />);
    const card = root.querySelector('[data-block-id="todo"]') as HTMLElement;
    const target = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const outside = document.createElement("div");
    document.body.appendChild(outside);
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = (x) => x < 20 ? target : outside;

    try {
      card.dispatchEvent(pointer("pointerdown", 0, 0, 41));
      window.dispatchEvent(pointer("pointermove", 12, 0, 41));
      window.dispatchEvent(pointer("pointerup", 30, 0, 41));

      expect(doc.byId.todo.raw.split("\n")[0]).toBe("TODO Write tests");
      expect(document.body.querySelector(".sheet-board-drag-ghost")).toBeNull();
    } finally {
      document.elementFromPoint = prevElementFromPoint;
      outside.remove();
      dispose();
    }
  });

  it("ignores move and release events from a pointer that does not own the drag", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <Block id="board" />);
    const card = root.querySelector('[data-block-id="todo"]') as HTMLElement;
    const target = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => target;

    try {
      card.dispatchEvent(pointer("pointerdown", 0, 0, 51));
      window.dispatchEvent(pointer("pointermove", 12, 0, 51));
      window.dispatchEvent(pointer("pointermove", 20, 0, 52));
      window.dispatchEvent(pointer("pointerup", 20, 0, 52));

      expect(doc.byId.todo.raw.split("\n")[0]).toBe("TODO Write tests");
      expect(document.body.querySelector(".sheet-board-drag-ghost")).not.toBeNull();

      window.dispatchEvent(pointer("pointerup", 12, 0, 51));
      expect(doc.byId.todo.raw.split("\n")[0]).toBe("DOING Write tests");
      expect(document.body.querySelector(".sheet-board-drag-ghost")).toBeNull();
    } finally {
      document.elementFromPoint = prevElementFromPoint;
      dispose();
    }
  });

  it("cancels without mutation when the initiating pointer loses capture", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <Block id="board" />);
    const card = root.querySelector('[data-block-id="todo"]') as HTMLElement;
    const target = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => target;

    try {
      card.dispatchEvent(pointer("pointerdown", 0, 0, 61));
      window.dispatchEvent(pointer("pointermove", 12, 0, 61));
      card.dispatchEvent(pointer("lostpointercapture", 12, 0, 61));

      expect(document.body.querySelector(".sheet-board-drag-ghost")).toBeNull();
      expect(document.body.classList.contains("sheet-board-dragging")).toBe(false);
      window.dispatchEvent(pointer("pointerup", 12, 0, 61));
      expect(doc.byId.todo.raw.split("\n")[0]).toBe("TODO Write tests");
    } finally {
      document.elementFromPoint = prevElementFromPoint;
      dispose();
    }
  });

  it("shows a floating drag ghost and lets the shared dispatcher cancel it before a lower layer", () => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <Block id="board" />);
    const uninstall = installKeybindings();
    let lowerDismissals = 0;
    const lowerRoot = document.createElement("button");
    document.body.append(lowerRoot);
    const unregisterLower = registerTransientLayer({
      id: "sheet-board-drag-lower",
      root: () => lowerRoot,
      dismiss: () => { lowerDismissals += 1; return true; },
    });
    const card = root.querySelector('[data-block-id="todo"]') as HTMLElement;
    const target = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => target;
    const releasePointerCapture = vi.fn();
    card.setPointerCapture = vi.fn();
    card.hasPointerCapture = vi.fn(() => true);
    card.releasePointerCapture = releasePointerCapture;

    card.dispatchEvent(pointer("pointerdown", 0, 0));
    window.dispatchEvent(pointer("pointermove", 12, 0));

    const ghost = document.body.querySelector(".sheet-board-drag-ghost") as HTMLElement | null;
    expect(ghost).not.toBeNull();
    expect(ghost!.style.transform).toContain("translate(22px, 10px)");
    expect(document.body.classList.contains("sheet-board-dragging")).toBe(true);

    const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    card.dispatchEvent(escape);

    expect(escape.defaultPrevented).toBe(true);
    expect(lowerDismissals).toBe(0);
    expect(document.body.querySelector(".sheet-board-drag-ghost")).toBeNull();
    expect(document.body.classList.contains("sheet-board-dragging")).toBe(false);
    expect(doc.byId.todo.raw.split("\n")[0]).toBe("TODO Write tests");
    expect(releasePointerCapture).toHaveBeenCalledWith(1);
    card.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    expect(lowerDismissals).toBe(1);

    document.elementFromPoint = prevElementFromPoint;
    unregisterLower();
    uninstall();
    dispose();
  });

  it.each([
    ["composing", { composing: true, keyCode: undefined }],
    ["keyCode 229", { composing: false, keyCode: 229 }],
  ] as const)("keeps a board drag active for %s Escape", (_name, variant) => {
    loadBoardDoc();
    const { root, dispose } = mount(() => <Block id="board" />);
    const uninstall = installKeybindings();
    const card = root.querySelector('[data-block-id="todo"]') as HTMLElement;
    const target = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => target;

    card.dispatchEvent(pointer("pointerdown", 0, 0));
    window.dispatchEvent(pointer("pointermove", 12, 0));
    expect(document.body.querySelector(".sheet-board-drag-ghost")).not.toBeNull();

    const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    if (variant.composing) Object.defineProperty(event, "isComposing", { value: true });
    if (variant.keyCode != null) Object.defineProperty(event, "keyCode", { value: variant.keyCode });
    card.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
    expect(document.body.querySelector(".sheet-board-drag-ghost")).not.toBeNull();

    card.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    expect(document.body.querySelector(".sheet-board-drag-ghost")).toBeNull();
    document.elementFromPoint = prevElementFromPoint;
    uninstall();
    dispose();
  });

  it("cancels and cleans a drag when a query refresh hides the card beyond pagination", () => {
    const targetId = "live-target";
    const filler = Array.from({ length: 270 }, (_, i) => ({
      id: `f${i}`, raw: `TODO Filler ${i}`, marker: "TODO", collapsed: false, children: [],
    }));
    const targetDto = { id: targetId, raw: "TODO Target", marker: "TODO", collapsed: false, children: [] };
    const remote: FeedPage = {
      name: "Remote", kind: "page", title: "Remote", preBlock: null, roots: [targetId],
      format: "md", readOnly: false, guide: false,
    };
    setDoc({
      byId: {
        board: node("board", "Board\ntine.view:: board\ntine.group-by:: state", null),
        [targetId]: { ...node(targetId, targetDto.raw, null), page: remote.name },
      },
      pages: [page(["board"]), remote], feed: ["Sheet"], loaded: true,
    });
    const group = (blocks: typeof filler): RefGroup[] => [{ page: remote.name, kind: "page", blocks }];
    const [groups, setGroups] = createSignal(group([targetDto, ...filler]));
    const { root, dispose } = mount(() => (
      <SheetBoard ownerId="board" rowSource="query" groupBy="state" groups={groups()} />
    ));
    const card = root.querySelector(`[data-block-id="${targetId}"]`) as HTMLElement;
    const targetColumn = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => targetColumn;

    card.dispatchEvent(pointer("pointerdown", 0, 0));
    window.dispatchEvent(pointer("pointermove", 12, 0));
    expect(document.body.querySelector(".sheet-board-drag-ghost")).not.toBeNull();

    setGroups(group([...filler.slice(0, 260), targetDto, ...filler.slice(260)]));
    expect(root.querySelector(`[data-block-id="${targetId}"]`)).toBeNull();
    expect(document.body.querySelector(".sheet-board-drag-ghost")).toBeNull();
    expect(document.body.classList.contains("sheet-board-dragging")).toBe(false);
    const before = doc.byId[targetId].raw;
    window.dispatchEvent(pointer("pointermove", 20, 0));
    window.dispatchEvent(pointer("pointerup", 20, 0));
    expect(doc.byId[targetId].raw).toBe(before);
    expect(document.body.querySelector(".sheet-board-drag-ghost")).toBeNull();

    document.elementFromPoint = prevElementFromPoint;
    dispose();
  });

  it("cancels and cleans a drag when group-by changes before pointerup", () => {
    loadBoardDoc();
    const [group, setGroup] = createSignal("state");
    const { root, dispose } = mount(() => (
      <SheetBoard ownerId="board" rowSource="children" groupBy={group()} />
    ));
    const card = root.querySelector('[data-block-id="todo"]') as HTMLElement;
    const targetColumn = root.querySelector('[data-board-col="1"]') as HTMLElement;
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => targetColumn;

    card.dispatchEvent(pointer("pointerdown", 0, 0));
    window.dispatchEvent(pointer("pointermove", 12, 0));
    expect(document.body.querySelector(".sheet-board-drag-ghost")).not.toBeNull();

    setGroup("priority");
    expect(document.body.querySelector(".sheet-board-drag-ghost")).toBeNull();
    expect(document.body.classList.contains("sheet-board-dragging")).toBe(false);
    const before = doc.byId.todo.raw;
    window.dispatchEvent(pointer("pointermove", 20, 0));
    window.dispatchEvent(pointer("pointerup", 20, 0));
    expect(doc.byId.todo.raw).toBe(before);
    expect(document.body.querySelector(".sheet-board-drag-ghost")).toBeNull();

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

  it("groups tag boards in one row walk while preserving multi-tag membership and order", () => {
    loadTagBoard({
      alphaBeta: "Alpha beta #alpha #beta",
      gamma: "Gamma #gamma",
      none: "No tags",
      betaDeltaAlpha: "Beta delta alpha #beta #delta #alpha",
      delta: "Delta #delta",
      gammaBeta: "Gamma beta #gamma #beta",
    });
    const walked: string[] = [];
    __sheetBoardTestHooks.onGroupingRowWalk = (rowId) => walked.push(rowId);

    const { root, dispose } = mount(() => <Block id="board" />);

    expect([...root.querySelectorAll(".sheet-board-header")].map((h) => h.textContent?.trim())).toEqual([
      "alpha2",
      "beta3",
      "gamma2",
      "delta2",
      "(none)1",
    ]);
    expect(boardColumnBlockIds(root, 0)).toEqual(["alphaBeta", "betaDeltaAlpha"]);
    expect(boardColumnBlockIds(root, 1)).toEqual(["alphaBeta", "betaDeltaAlpha", "gammaBeta"]);
    expect(boardColumnBlockIds(root, 2)).toEqual(["gamma", "gammaBeta"]);
    expect(boardColumnBlockIds(root, 3)).toEqual(["betaDeltaAlpha", "delta"]);
    expect(boardColumnBlockIds(root, 4)).toEqual(["none"]);
    expect(walked).toEqual(["alphaBeta", "gamma", "none", "betaDeltaAlpha", "delta", "gammaBeta"]);

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

  it("keeps a multi-tag card bound to the selected tag column for keyboard moves", () => {
    loadTagBoard({ todo: "Card #alpha #beta", other: "Other #gamma" });
    const { root, dispose } = mount(() => <Block id="board" />);
    const betaColumn = [...root.querySelectorAll<HTMLElement>(".sheet-board-column")].find(
      (column) => column.querySelector(".sheet-board-header span")?.textContent === "beta"
    )!;
    const betaCard = betaColumn.querySelector<HTMLElement>('[data-block-id="todo"]')!;

    betaCard.click();
    const selected = cellSel();
    expect(selected?.kind === "cell" ? { col: selected.col, rowId: selected.rowId, columnId: selected.columnId } : null)
      .toEqual({ col: 1, rowId: "todo", columnId: JSON.stringify("beta") });

    const otherBefore = doc.byId.other.raw;
    expect(handleCellSelectionKey(keydown("ArrowRight", { ctrlKey: true }))).toBe(true);
    expect(doc.byId.todo.raw).toContain("#alpha");
    expect(doc.byId.todo.raw).not.toContain("#beta");
    expect(doc.byId.todo.raw).toContain("#gamma");
    expect(doc.byId.other.raw).toBe(otherBefore);
    dispose();
  });

  it("keeps Board editor ownership stable across numeric column reorder but not membership change", () => {
    const beta = { gridId: "board", surfaceId: "main", rowId: "todo", columnId: JSON.stringify("beta"), row: 0, col: 1 };
    expect(cellOwner(beta)).toBe(cellOwner({ ...beta, col: 4 }));
    expect(cellOwner(beta)).not.toBe(cellOwner({ ...beta, columnId: JSON.stringify("gamma"), col: 1 }));
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

  it("renders exactly one board when text and a query macro share the same line", async () => {
    setDoc({
      byId: {
        query: node(
          "query",
          "Current work {{query (todo TODO)}}\ntine.view:: board\ntine.group-by:: state",
          null
        ),
        todo: node("todo", "TODO From inline query", null),
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
    expect(root.textContent).toContain("From inline query");

    dispose();
  });
});

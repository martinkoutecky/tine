import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import * as historyStoreModule from "./store";
import * as editorControllerModule from "./editorController";
import { paletteCommands } from "./keybindings";
import {
  focusPane,
  focusedPaneId,
  mainRouter,
  paneRouter,
  resetPaneLayoutToSingle,
  splitPane,
} from "./panes";
import {
  rightSidebar,
  rightSidebarOpen,
  setRightSidebar,
  setRightSidebarOpen,
  setToasts,
  toasts,
} from "./ui";
import type { BlockDto, PageDto } from "./types";
import { initParser } from "./render/parse";

type HistoryStoreApi = typeof historyStoreModule & {
  historyPageOnlyMode(): boolean;
  toggleUndoRedoMode(): "Page only" | "Global";
};
type HistoryEditorTarget = {
  blockId: string;
  owner: string | null;
  surface: string;
  selection: () => { start: number; end: number };
  focused?: () => boolean;
};
type PendingHistoryEditorRestore = {
  blockId: string;
  selectionStart: number;
  selectionEnd: number;
  owner: string | null;
  surface: string;
};
type EditorControllerApi = typeof editorControllerModule & {
  registerHistoryEditorTarget(target: HistoryEditorTarget): () => void;
  pendingHistoryEditorRestore(): PendingHistoryEditorRestore | null;
  clearPendingHistoryEditorRestore(): void;
};

const store = historyStoreModule as HistoryStoreApi;
const editor = editorControllerModule as EditorControllerApi;
let cleanups: (() => void)[] = [];

beforeAll(() => initParser());

function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  return { id, raw, collapsed: false, children };
}

function page(name: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "page", title: name, pre_block: null, blocks, format: "md" };
}

function dtoBytes(name: string): string {
  return JSON.stringify(store.pageToDto(name));
}

function setRoute(name: string) {
  mainRouter().replaceActiveRoute({ kind: "page", name, pageKind: "page" });
}

function editTarget(
  blockId: string,
  selection: { start: number; end: number },
  owner = "owner-a",
  surface = "pane:main",
) {
  editor.startEditing(blockId, selection.start, owner, surface);
  cleanups.push(editor.registerHistoryEditorTarget({
    blockId,
    owner,
    surface,
    selection: () => selection,
    focused: () => true,
  }));
}

beforeEach(() => {
  cleanups = [];
  store.resetStore();
  resetPaneLayoutToSingle({
    tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
    activeIndex: 0,
  });
  setRightSidebar([]);
  setRightSidebarOpen(false);
  setToasts([]);
  editor.clearPendingHistoryEditorRestore?.();
  if (store.historyPageOnlyMode?.()) store.toggleUndoRedoMode();
});

afterEach(() => {
  for (const cleanup of cleanups) cleanup();
  store.resetStore();
  setRightSidebar([]);
  setRightSidebarOpen(false);
  setToasts([]);
});

describe("history parity", () => {
  it("page-only undo/redo removes only A's newest interleaved raw/structural entries", () => {
    store.loadFeed([
      page("A", [block("a", "alpha")]),
      page("B", [block("b", "beta")]),
    ]);
    setRoute("B");
    editor.startEditing("a", 2, "owner-a", "pane:main"); // editing page wins over route page

    store.setRaw("a", "alpha raw");
    store.splitBlock("b", 2);
    store.splitBlock("a", 5);
    store.setRaw("b", "B head");

    const bBytes = dtoBytes("B");
    const aEdited = dtoBytes("A");
    store.toggleUndoRedoMode();

    for (let cycle = 0; cycle < 2; cycle++) {
      editor.startEditing("a", 2, "owner-a", "pane:main");
      store.undo(); // A structural entry, even though B's raw entry is globally newer
      expect(dtoBytes("B")).toBe(bBytes);
      expect(mainRouter().route()).toEqual({ kind: "page", name: "B", pageKind: "page" });
      editor.startEditing("a", 2, "owner-a", "pane:main");
      store.undo(); // A raw entry; B's structural + raw entries remain in place
      expect(dtoBytes("B")).toBe(bBytes);
      expect(mainRouter().route()).toEqual({ kind: "page", name: "B", pageKind: "page" });
      expect(store.pageToDto("A")?.blocks.map((item) => item.raw)).toEqual(["alpha"]);

      editor.startEditing("a", 2, "owner-a", "pane:main");
      store.redo();
      expect(dtoBytes("B")).toBe(bBytes);
      editor.startEditing("a", 2, "owner-a", "pane:main");
      store.redo();
      expect(dtoBytes("B")).toBe(bBytes);
      expect(dtoBytes("A")).toBe(aEdited);
    }
  });

  it("defaults to global selection and the mode toggle switches to route-page selection", () => {
    store.loadFeed([
      page("A", [block("a", "alpha")]),
      page("B", [block("b", "beta")]),
    ]);
    setRoute("A");
    store.setRaw("a", "A edited");
    store.setRaw("b", "B edited");

    expect(store.historyPageOnlyMode()).toBe(false);
    store.undo();
    expect(store.doc.byId.b.raw).toBe("beta");
    expect(store.doc.byId.a.raw).toBe("A edited");
    store.redo();

    expect(store.toggleUndoRedoMode()).toBe("Page only");
    store.undo();
    expect(store.doc.byId.a.raw).toBe("alpha");
    expect(store.doc.byId.b.raw).toBe("B edited");
  });

  it("registers a palette-only mode command and reports the resulting mode", () => {
    const command = paletteCommands().find((item) => item.id === "editor/toggle-undo-redo-mode");
    expect(command).toMatchObject({ label: "Toggle undo/redo mode", binding: "" });

    command!.run();
    expect(store.historyPageOnlyMode()).toBe(true);
    expect(toasts().at(-1)?.message).toBe("Undo/redo mode: Page only");
  });

  it("captures raw-entry route/sidebar/editor context and restores a clamped selection request", () => {
    store.loadFeed([
      page("A", [block("a", "abc")]),
      page("B", [block("b", "beta")]),
    ]);
    setRoute("A");
    const historyPane = splitPane("main", "row");
    expect(historyPane).not.toBeNull();
    paneRouter(historyPane!).replaceActiveRoute({ kind: "page", name: "A", pageKind: "page" });
    setRightSidebar([{ kind: "page", name: "A", pageKind: "page" }]);
    setRightSidebarOpen(true);
    const selection = { start: 2, end: 99 };
    editTarget("a", selection, "owner-a", `pane:${historyPane}`);

    store.setRaw("a", "edited content");
    editor.endEdit("page-navigation");
    focusPane("main");
    setRoute("B");
    setRightSidebar([{ kind: "page", name: "B", pageKind: "page" }]);
    setRightSidebarOpen(false);

    store.undo();

    expect(store.doc.byId.a.raw).toBe("abc");
    expect(focusedPaneId()).toBe(historyPane);
    expect(paneRouter(historyPane!).route()).toEqual({ kind: "page", name: "A", pageKind: "page" });
    expect(rightSidebarOpen()).toBe(true);
    expect(rightSidebar()).toEqual([{ kind: "page", name: "A", pageKind: "page" }]);
    expect(editor.pendingHistoryEditorRestore()).toEqual({
      blockId: "a",
      selectionStart: 2,
      selectionEnd: 3,
      owner: "owner-a",
      surface: `pane:${historyPane}`,
    });

    // Snapshot entries carry the same context payload as raw typing entries.
    selection.start = 1;
    selection.end = 50;
    editor.startEditing("a", 1, "owner-a", `pane:${historyPane}`);
    store.splitBlock("a", 1);
    editor.endEdit("page-navigation");
    focusPane("main");
    setRoute("B");
    setRightSidebar([{ kind: "page", name: "B", pageKind: "page" }]);
    setRightSidebarOpen(false);

    store.undo();
    expect(store.doc.byId.a.raw).toBe("abc");
    expect(focusedPaneId()).toBe(historyPane);
    expect(rightSidebarOpen()).toBe(true);
    expect(rightSidebar()).toEqual([{ kind: "page", name: "A", pageKind: "page" }]);
    expect(editor.pendingHistoryEditorRestore()).toMatchObject({
      blockId: "a",
      selectionStart: 1,
      selectionEnd: 3,
      surface: `pane:${historyPane}`,
    });
  });

  it("keeps data replay and stack order intact when saved route/block context is missing", () => {
    store.loadFeed([page("B", [block("b", "before")])]);
    const before = dtoBytes("B");
    setRoute("Deleted page");
    editTarget("deleted-block", { start: 7, end: 12 }, "gone-owner", "gone-surface");

    store.setRaw("b", "after");
    const after = dtoBytes("B");
    editor.endEdit("page-navigation");
    setRoute("B");

    expect(() => store.undo()).not.toThrow();
    expect(dtoBytes("B")).toBe(before);
    expect(mainRouter().route()).toEqual({ kind: "page", name: "B", pageKind: "page" });
    expect(editor.pendingHistoryEditorRestore()).toBeNull();

    expect(() => store.redo()).not.toThrow();
    expect(dtoBytes("B")).toBe(after);
    expect(mainRouter().route()).toEqual({ kind: "page", name: "B", pageKind: "page" });
  });
});

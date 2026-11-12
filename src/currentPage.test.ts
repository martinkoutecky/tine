import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Route } from "./routeTypes";

const state = vi.hoisted(() => ({
  focused: true, window: "main", route: { kind: "page", name: "日本 / A?#%", pageKind: "page" } as Route,
  editing: null as string | null, element: null as Element | null,
  page: "日本 / A?#%", sourcePage: "日本 / A?#%", generation: 7,
  authored: null as string | null,
}));
const api = vi.hoisted(() => ({
  onCurrentPageRequest: vi.fn(), currentPageAnswer: vi.fn(async () => {}),
  identity: vi.fn(),
}));
vi.mock("./backend", () => ({ backend: () => ({ tineLinks: api }) }));
vi.mock("./binding", () => ({ captureBinding: () => ({ backendGeneration: state.generation }) }));
vi.mock("./panes", () => ({ focusedRouterOf: vi.fn(() => ({ paneId: "focused-pane", route: () => state.route })) }));
vi.mock("./windowRealm", () => ({
  activeWindow: () => ({ document: { hasFocus: () => state.focused } }),
  activeWindowId: () => state.window, activeElement: () => state.element,
}));
vi.mock("./editorController", () => ({ editingId: () => state.editing }));
vi.mock("./document", () => ({ node: () => ({ page: state.sourcePage }), authoredBlockId: () => state.authored }));
import { currentPage, subscribeCurrentPage } from "./currentPage";
import { focusedRouterOf } from "./panes";

const uuid = "11111111-1111-4111-8111-111111111111";
/** Focus the editor of a block whose text carries `authored` as its `id::` (null: none). */
function focusEditor(pane = "focused-pane", authored: string | null = uuid) {
  state.editing = "runtime-key";
  state.authored = authored;
  state.element = {
    closest: (selector: string) => ({ getAttribute: (key: string) =>
      selector === "[data-pane-id]" ? pane : selector === "[data-page-name]" ? state.page : state.editing }),
  } as unknown as Element;
}
beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(state, { focused: true, window: "main", editing: null, element: null, authored: null,
    page: "日本 / A?#%", sourcePage: "日本 / A?#%", generation: 7,
    route: { kind: "page", name: "日本 / A?#%", pageKind: "page" } });
});

describe("current page snapshot", () => {
  it("answers the focused pane with the existing page URL producer and no identity I/O", () => {
    state.window = "ws-main-2";
    expect(currentPage()).toEqual({ kind: "current-page", name: state.page,
      url: "tine://page/%E6%97%A5%E6%9C%AC%20%2F%20A%3F%23%25", block_url: null });
    expect(api.identity).not.toHaveBeenCalled();
    expect(focusedRouterOf).toHaveBeenCalledExactlyOnceWith("ws-main-2");
  });
  it("includes only the editing block with focus in the active pane", () => {
    focusEditor();
    expect(currentPage()).toMatchObject({ block_url: `tine://block/${uuid}` });
    focusEditor("other-pane");
    expect(currentPage()).toMatchObject({ block_url: null });
    state.element = null;
    expect(currentPage()).toMatchObject({ block_url: null });
  });
  it("answers the last-focused window's page while another app (the asking terminal) has focus", () => {
    state.focused = false;
    expect(currentPage()).toMatchObject({ kind: "current-page", name: state.page });
  });
  it("prints no block link for a block without an authored id, whatever its runtime key", () => {
    focusEditor("focused-pane", null);
    expect(currentPage()).toMatchObject({ kind: "current-page", name: state.page, block_url: null });
    expect(api.identity).not.toHaveBeenCalled();
  });
  it("prints no block link for an authored id that is not a UUID", () => {
    focusEditor("focused-pane", "not-a-uuid");
    expect(currentPage()).toMatchObject({ kind: "current-page", block_url: null });
  });
  it.each(["all-pages", "query", "pdf", "conflicts"])("reports the %s view as a typed absence", (kind) => {
    state.route = { kind } as Route;
    expect(currentPage()).toEqual({ kind: "no-current-page", reason: "not-page" });
  });
  it("identifies the journal day under the editing caret, otherwise the journals view", () => {
    state.route = { kind: "journals" };
    expect(currentPage()).toEqual({ kind: "no-current-page", reason: "journals" });
    state.page = "Nov 10th, 2026";
    state.sourcePage = state.page;
    focusEditor();
    expect(currentPage()).toMatchObject({ kind: "current-page", name: state.page, block_url: `tine://block/${uuid}` });
  });
  it("keeps the journal day when the caret edits a block embedded from another page", () => {
    state.route = { kind: "journals" };
    state.page = "Nov 10th, 2026";
    state.sourcePage = "Embedded source";
    focusEditor();
    expect(currentPage()).toMatchObject({ name: state.page, block_url: `tine://block/${uuid}` });
  });
  it("answers once and rejects a request from an old graph binding", async () => {
    let receive!: (request: { id: number; binding_generation: number }) => void;
    const stop = vi.fn();
    api.onCurrentPageRequest.mockImplementation(async (cb) => { receive = cb; return stop; });
    const dispose = subscribeCurrentPage();
    await vi.waitFor(() => expect(receive).toBeDefined());
    receive({ id: 1, binding_generation: 6 });
    expect(api.currentPageAnswer).not.toHaveBeenCalled();
    receive({ id: 2, binding_generation: 7 });
    expect(api.currentPageAnswer).toHaveBeenCalledExactlyOnceWith(2, currentPage(), 7);
    dispose();
    await vi.waitFor(() => expect(stop).toHaveBeenCalledOnce());
  });
  it("disposes a subscription that finishes registering after unmount", async () => {
    let finish!: (stop: () => void) => void;
    const stop = vi.fn();
    api.onCurrentPageRequest.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const dispose = subscribeCurrentPage();
    dispose();
    finish(stop);
    await vi.waitFor(() => expect(stop).toHaveBeenCalledOnce());
  });
});

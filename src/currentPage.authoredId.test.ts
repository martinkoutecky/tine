// CURRENT-PAGE-LINKS (D-20261111-QP2): `tine page current` prints a block link
// only for a block whose text carries an authored `id::`. The loaded document
// and the real parser answer; a loaded block's runtime UUID is not an address.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import type { Route } from "./routeTypes";

const state = vi.hoisted(() => ({ editing: null as string | null, element: null as Element | null }));
vi.mock("./panes", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./panes")>()),
  focusedRouterOf: () => ({ paneId: "pane", route: () => ({ kind: "page", name: "Loaded", pageKind: "page" } as Route) }),
}));
vi.mock("./windowRealm", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./windowRealm")>()),
  activeWindowId: () => "main", activeElement: () => state.element,
}));
vi.mock("./editorController", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./editorController")>()), editingId: () => state.editing,
}));
import { currentPage as snapshot } from "./currentPage";
import * as parser from "./render/parse";
import { doc, setDoc } from "./document/model";
import { blockExternalId } from "./document";
import { loadSingle, resetStore } from "./document/workingSet";
import { clearSeededFacets } from "./render/facets";

const authoredId = "92500000-0000-4000-8000-000000000001";
const currentPage = () => snapshot("11111111-1111-4111-8111-111111111111");
const runtimeId = "7e57ab1e-0000-4000-8000-000000000002";
beforeAll(() => parser.initParser());
afterEach(() => { resetStore(); clearSeededFacets(); state.editing = null; state.element = null; });

function load(blocks: { id: string; raw: string }[]) {
  loadSingle({
    name: "Loaded", title: "Loaded", kind: "page", pre_block: null, format: "md",
    blocks: blocks.map((b) => ({ ...b, has_id: b.raw.includes("id::"), collapsed: false, children: [] })),
  });
}
function editing(key: string) {
  state.editing = key;
  state.element = {
    closest: (selector: string) => ({ getAttribute: (name: string) =>
      selector === "[data-pane-id]" ? "pane" : name === "data-block-id" ? key
        // What Block.tsx renders: the authored id, else the runtime key.
        : name === "data-block-ref" ? blockExternalId(key) : null }),
  } as unknown as Element;
}

it("links a block that carries an authored id, and only that block", () => {
  load([{ id: runtimeId, raw: `Authored\nid:: ${authoredId}` }, { id: "7e57ab1e-0000-4000-8000-000000000003", raw: "Plain" }]);
  editing(runtimeId);
  expect(currentPage()).toMatchObject({ kind: "current-page", name: "Loaded", block_url: `tine://block/${authoredId}` });
});

it("prints no block link for a loaded block without an id::, though its runtime UUID looks like one", () => {
  load([{ id: runtimeId, raw: "Plain" }]);
  editing(runtimeId);
  expect(currentPage()).toMatchObject({ kind: "current-page", name: "Loaded", block_url: null });
});

it("prints no block link for a newly inserted block (transient key)", () => {
  load([{ id: "b-new-block", raw: "" }]);
  editing("b-new-block");
  expect(currentPage()).toMatchObject({ kind: "current-page", block_url: null });
});

it("follows the block once a reference stamped its id:: into the text", () => {
  load([{ id: "b-new-block", raw: "Fresh" }]);
  editing("b-new-block");
  expect(currentPage()).toMatchObject({ block_url: null });
  setDoc("byId", "b-new-block", "raw", `Fresh\nid:: ${authoredId}`);
  expect(doc.byId["b-new-block"].raw).toContain(authoredId);
  expect(currentPage()).toMatchObject({ block_url: `tine://block/${authoredId}` });
});

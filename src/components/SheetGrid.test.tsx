import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { resetStore, setDoc, type Node, type FeedPage } from "../store";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function page(format: "md" | "org", roots: string[]): FeedPage {
  return {
    name: format === "org" ? "Org Sheet" : "Sheet",
    kind: "page",
    title: format === "org" ? "Org Sheet" : "Sheet",
    preBlock: null,
    roots,
    format,
    readOnly: false,
  };
}

function node(
  id: string,
  raw: string,
  pageName: string,
  parent: string | null,
  children: string[] = []
): Node {
  return { id, raw, collapsed: false, parent, page: pageName, children };
}

function loadMdSheetDoc() {
  const pageName = "Sheet";
  setDoc({
    byId: {
      grid: node(
        "grid",
        "Grid parent\ntine.view:: grid\ntine.header:: true\ntine.col-widths:: 0=120;2=88",
        pageName,
        null,
        ["r1", "r2", "r3"]
      ),
      r1: node("r1", "", pageName, "grid", ["h1", "h2", "h3"]),
      h1: node("h1", "Name", pageName, "r1"),
      h2: node("h2", "Status", pageName, "r1"),
      h3: node("h3", "Notes", pageName, "r1"),
      r2: node("r2", "", pageName, "grid", ["c1", "c2", "c3"]),
      c1: node("c1", "TODO Ship [[Page Ref]]", pageName, "r2"),
      c2: node("c2", "Nested grid\ntine.view:: grid", pageName, "r2", ["nr1", "nr2"]),
      c3: node("c3", "Tail", pageName, "r2"),
      nr1: node("nr1", "", pageName, "c2", ["n11", "n12"]),
      n11: node("n11", "inner A", pageName, "nr1"),
      n12: node("n12", "inner B", pageName, "nr1"),
      nr2: node("nr2", "", pageName, "c2", ["n21"]),
      n21: node("n21", "inner C", pageName, "nr2"),
      r3: node("r3", "", pageName, "grid", ["c4"]),
      c4: node("c4", "ragged", pageName, "r3"),
      plain: node("plain", "Plain parent", pageName, null, ["plain-child"]),
      "plain-child": node("plain-child", "plain child", pageName, "plain"),
    },
    pages: [page("md", ["grid", "plain"])],
    feed: [pageName],
    loaded: true,
  });
}

function loadOrgSheetDoc() {
  const pageName = "Org Sheet";
  setDoc({
    byId: {
      "org-grid": node(
        "org-grid",
        "Org grid\n:PROPERTIES:\n:tine.view: grid\n:tine.header: true\n:END:",
        pageName,
        null,
        ["or1"]
      ),
      or1: node("or1", "", pageName, "org-grid", ["oh1", "oh2"]),
      oh1: node("oh1", "Org A", pageName, "or1"),
      oh2: node("oh2", "Org B", pageName, "or1"),
    },
    pages: [page("org", ["org-grid"])],
    feed: [pageName],
    loaded: true,
  });
}

describe("SheetGrid", () => {
  it("renders a read-only positional grid with widths, holes, headers, hidden tine props, and nested grids", () => {
    loadMdSheetDoc();
    const { root, dispose } = mount(() => <Block id="grid" />);

    const grid = root.querySelector(".block-sheet-container > .sheet-grid") as HTMLElement | null;
    expect(grid).not.toBeNull();
    expect(grid!.style.gridTemplateColumns).toBe("120px max-content 88px");
    expect(grid!.style.gridTemplateColumns.trim().split(/\s+/)).toHaveLength(3);
    expect(grid!.querySelectorAll(":scope > .sheet-hole")).toHaveLength(2);
    expect(grid!.querySelectorAll(":scope > .sheet-header-cell")).toHaveLength(3);
    expect(root.textContent).not.toContain("tine.view");
    expect(root.querySelector(".sheet-cell .sheet-grid")).not.toBeNull();

    dispose();
  });

  it("leaves non-grid blocks on the existing vertical children renderer", () => {
    loadMdSheetDoc();
    const { root, dispose } = mount(() => <Block id="plain" />);

    expect(root.querySelector(".block-children-container")).not.toBeNull();
    expect(root.querySelector(".sheet-grid")).toBeNull();
    expect(root.textContent).toContain("plain child");

    dispose();
  });

  it("detects org property drawers through format-aware facets", () => {
    loadOrgSheetDoc();
    const { root, dispose } = mount(() => <Block id="org-grid" />);

    expect(root.querySelector(".sheet-grid")).not.toBeNull();
    expect(root.querySelectorAll(".sheet-header-cell")).toHaveLength(2);
    expect(root.textContent).not.toContain("tine.view");

    dispose();
  });
});

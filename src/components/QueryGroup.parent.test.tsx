import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import type { BlockDto, PageDto, RefGroup } from "../types";
import { QueryGroups } from "./QueryGroup";

// GH #619 item 1 (OG parity): block.cljs `custom-query-results` groups a page's
// matches by :block/parent, so `A -> TODO One, TODO Two` shows A once.

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

const leaf = (id: string, raw: string, breadcrumb: string[]): BlockDto => ({
  id,
  raw,
  collapsed: false,
  children: [],
  breadcrumb,
});

const crumbs = (root: HTMLElement) =>
  [...root.querySelectorAll(".ref-breadcrumb")].map((c) => c.textContent?.replace(/\s+/g, "").trim());

describe("query results group matches by parent (GH #619 item 1)", () => {
  it("shows a shared parent once for sibling matches, before the page is loaded", async () => {
    const group: RefGroup = {
      page: "Tasks",
      kind: "page",
      blocks: [
        leaf("one", "TODO One", ["A"]),
        leaf("other", "TODO Elsewhere", ["B"]),
        leaf("two", "TODO Two", ["A"]),
      ],
    } as RefGroup;
    const { root, dispose } = mount(() => <QueryGroups groups={() => new Map([["Tasks", group]])} />);
    try {
      await expect.poll(() => root.textContent).toContain("Two");
      expect(crumbs(root)).toEqual(["A", "B"]);
    } finally {
      dispose();
    }
  });

  it("shows a shared parent once once the source page is loaded (hydrated path)", async () => {
    const one = leaf("one", "TODO One", []);
    const two = leaf("two", "TODO Two", []);
    const page: PageDto = {
      name: "Tasks",
      title: "Tasks",
      kind: "page",
      pre_block: null,
      blocks: [
        { id: "a", raw: "A", collapsed: false, children: [one, two] },
        { id: "b", raw: "B", collapsed: false, children: [leaf("other", "TODO Elsewhere", [])] },
      ],
    };
    loadSingle(page);
    const group: RefGroup = {
      page: "Tasks",
      kind: "page",
      blocks: [
        { ...one, breadcrumb: ["A"] },
        leaf("other", "TODO Elsewhere", ["B"]),
        { ...two, breadcrumb: ["A"] },
      ],
    } as RefGroup;
    const { root, dispose } = mount(() => <QueryGroups groups={() => new Map([["Tasks", group]])} />);
    try {
      await expect.poll(() => crumbs(root).length).toBe(2);
      expect(crumbs(root)).toEqual(["A", "B"]);
      const text = root.textContent ?? "";
      expect(text.indexOf("One")).toBeLessThan(text.indexOf("Two"));
      expect(text.indexOf("Two")).toBeLessThan(text.indexOf("Elsewhere"));
    } finally {
      dispose();
    }
  });
});

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { setDoc } from "../document/model";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";
import type { BlockDto, PageDto, RefGroup } from "../types";
import { QueryGroups } from "./QueryGroup";
import { LiveRefGroup } from "./LiveRefGroup";

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
  // GH #643 follow-up (Martin, journal query "**Queue**"): a trail renders its
  // parent's markup like the block itself (OG block.cljs `breadcrumb` map-inline),
  // both before and after the source page loads.
  it("renders a parent's markup in the trail, not the raw source", async () => {
    const rawParent = "**Queue** for [[Work]]";
    const before = mount(() => <QueryGroups groups={() => new Map([["Tasks", {
      page: "Tasks", kind: "page", blocks: [leaf("q1", "TODO One", [rawParent])],
    } as RefGroup]])} />);
    try {
      await expect.poll(() => before.root.querySelector(".ref-breadcrumb strong")?.textContent).toBe("Queue");
      expect(before.root.querySelector(".ref-breadcrumb")?.textContent).not.toContain("**");
      expect(before.root.querySelector(".ref-breadcrumb .page-ref")?.textContent).toContain("Work");
    } finally { before.dispose(); document.body.innerHTML = ""; }
    const one = leaf("q1", "TODO One", []);
    setDoc({ byId: {
      queue: { id: "queue", raw: rawParent, collapsed: false, parent: null, page: "Tasks", children: ["q1"] },
      q1: { id: "q1", raw: "TODO One", collapsed: false, parent: "queue", page: "Tasks", children: [] },
    }, pages: [{ id: "pages/Tasks.md", name: "Tasks", kind: "page", title: "Tasks", preBlock: null, roots: ["queue"], format: "md", readOnly: false, guide: false }], feed: ["Tasks"], loaded: true } as never);
    const rows = [{ ...one, breadcrumb: [rawParent] }];
    adoptResults(rows, captureResultsTicket());
    const after = mount(() => <LiveRefGroup page="Tasks" kind="page" path="pages/Tasks.md" blocks={rows} surface="query" showBreadcrumb eager />);
    try {
      await expect.poll(() => after.root.querySelector("button.ref-crumb strong")?.textContent).toBe("Queue");
      const crumb = after.root.querySelector(".ref-breadcrumb")!;
      expect(crumb.textContent).not.toContain("**");
      expect(crumb.querySelector(".page-ref")?.textContent).toContain("Work");
    } finally { after.dispose(); }
  });
});

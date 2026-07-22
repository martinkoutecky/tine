import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore, setDoc, type FeedPage, type Node as StoreNode } from "../store";
import type { BlockDto, RefGroup } from "../types";
import { Block } from "./Block";
import { LiveRefGroup } from "./LiveRefGroup";
import { RefBlocks } from "./RefBlocks";

const BEGIN_QUERY = `#+BEGIN_QUERY
{:title "Class pages"
 :query [:find (pull ?p [*])
         :where
         [?p :block/properties ?props]
         [(get ?props :class)]]}
#+END_QUERY`;

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  localStorage.clear();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function node(id: string, raw: string): StoreNode {
  return { id, raw, collapsed: false, parent: null, page: "Source", children: [] };
}

function page(roots: string[]): FeedPage {
  return {
    name: "Source",
    kind: "page",
    title: "Source",
    preBlock: null,
    roots,
    format: "md",
    readOnly: false,
    guide: false,
  };
}

function dto(id: string, raw: string): BlockDto {
  return { id, raw, collapsed: false, children: [] };
}

function seedQuery(): BlockDto {
  setDoc({
    byId: {
      query: node("query", BEGIN_QUERY),
      result: node("result", "A matching class page"),
    },
    pages: [page(["query", "result"])],
    feed: ["Source"],
    loaded: true,
  });
  const groups: RefGroup[] = [{
    page: "Source",
    kind: "page",
    blocks: [dto("result", "A matching class page")],
  }];
  vi.spyOn(backend(), "runAdvancedQuery").mockResolvedValue({
    groups,
    ran: ["page-property"],
    ignored: [],
    supported: true,
  });
  return dto("query", BEGIN_QUERY);
}

async function expectRenderedQuery(root: HTMLElement): Promise<void> {
  await vi.waitFor(() => expect(root.querySelector(".query-title")?.textContent).toBe("Class pages"));
  await vi.waitFor(() => expect(root.querySelectorAll(".query-table")).toHaveLength(1));
  expect(root.textContent).toContain("A matching class page");
  expect(root.textContent).not.toContain("#+BEGIN_QUERY");
  expect(root.textContent).not.toContain("#+END_QUERY");
  expect(root.textContent).not.toContain(":block/properties");
}

describe("terminated whole-block BEGIN_QUERY", () => {
  it("renders its authored title and table on the main page", async () => {
    seedQuery();
    const { root, dispose } = mount(() => <Block id="query" />);
    try {
      await expectRenderedQuery(root);
      expect(backend().runAdvancedQuery).toHaveBeenCalledWith(expect.any(String), "Source");
    } finally {
      dispose();
    }
  });

  it("renders through a hydrated Linked References group", async () => {
    const query = seedQuery();
    const { root, dispose } = mount(() => (
      <LiveRefGroup page="Source" kind="page" blocks={[query]} surface="ref" />
    ));
    try {
      await expectRenderedQuery(root);
    } finally {
      dispose();
    }
  });

  it("renders through the RefBlocks fallback", async () => {
    const query = seedQuery();
    const { root, dispose } = mount(() => <RefBlocks blocks={[query]} page="Source" pageKind="page" />);
    try {
      await expectRenderedQuery(root);
    } finally {
      dispose();
    }
  });

  it("fails visibly for a malformed payload without exposing raw delimiters", async () => {
    const malformed = dto("bad-query", "#+BEGIN_QUERY\n{:title \"Broken\" :query nope}\n#+END_QUERY");
    const runAdvanced = vi.spyOn(backend(), "runAdvancedQuery");
    const { root, dispose } = mount(() => <RefBlocks blocks={[malformed]} page="Source" pageKind="page" />);
    try {
      await vi.waitFor(() => expect(root.querySelector('[role="alert"]')?.textContent).toContain("Unsupported BEGIN_QUERY"));
      expect(root.textContent).not.toContain("#+BEGIN_QUERY");
      expect(root.textContent).not.toContain(":query nope");
      expect(runAdvanced).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("does not render a guessed partial result when the evaluator ignored a clause", async () => {
    const partialSource = BEGIN_QUERY
      .replace("Class pages", "Partial pages")
      .replace(":class)", ":partial-class)");
    const query = dto("partial-query", partialSource);
    vi.spyOn(backend(), "runAdvancedQuery").mockResolvedValue({
      groups: [{ page: "Source", kind: "page", blocks: [dto("result", "A matching class page")] }],
      ran: ["page-property"],
      ignored: ["pattern"],
      supported: true,
    });
    const { root, dispose } = mount(() => <RefBlocks blocks={[query]} page="Source" pageKind="page" />);
    try {
      await vi.waitFor(() => expect(root.querySelector('[role="alert"]')?.textContent).toContain("Unsupported BEGIN_QUERY"));
      expect(root.textContent).not.toContain("A matching class page");
      expect(root.textContent).not.toContain(":block/properties");
    } finally {
      dispose();
    }
  });

  it("leaves non-query custom blocks on the generic renderer", () => {
    const ordinary = dto("ordinary", "#+BEGIN_WIDGET\nwidget body\n#+END_WIDGET");
    const { root, dispose } = mount(() => <RefBlocks blocks={[ordinary]} page="Source" pageKind="page" />);
    try {
      expect(root.textContent).toContain("BEGIN_WIDGET");
      expect(root.textContent).toContain("widget body");
    } finally {
      dispose();
    }
  });
});

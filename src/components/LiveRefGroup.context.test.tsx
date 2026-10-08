import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { backend } from "../backend";
import { loadSingle } from "../document/workingSet";
import { doc, setDoc } from "../document/model";
import type { BlockDto, PageDto, PageRead } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";
import { applySidebarSession, rightSidebar, setRightSidebar } from "../ui";
import { route } from "../router";
import { UnlinkedReferences } from "./UnlinkedReferences";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setRightSidebar([]);
  applySidebarSession({ right: false, items: [] });
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function hierarchy(): { page: PageDto; result: BlockDto; sourceHit: BlockDto } {
  const grandchild: BlockDto = {
    id: "ref-grandchild",
    raw: "Grandchild body",
    collapsed: false,
    children: [],
  };
  const child: BlockDto = {
    id: "ref-child",
    raw: "Child body",
    collapsed: false,
    children: [grandchild],
  };
  const root: BlockDto = {
    id: "ref-root",
    raw: "Root [[Target]]",
    collapsed: false,
    children: [child],
    breadcrumb: ["One", "Two", "Three", "Four", "Five"],
  };
  const labels = ["One", "Two", "Three", "Four", "Five"];
  let sourceRoot = root;
  for (const [index, label] of [...labels].reverse().entries()) {
    sourceRoot = {
      id: `ancestor-${labels.length - index}`,
      raw: label,
      collapsed: false,
      children: [sourceRoot],
    };
  }
  return {
    page: {
      name: "Source",
      title: "Source",
      kind: "page",
      pre_block: null,
      blocks: [sourceRoot],
    },
    // Reference/query membership may be refreshed as a new object and may carry
    // only the hit root. The live hierarchy comes from the loaded source page.
    result: { ...root, children: [], breadcrumb: undefined },
    sourceHit: root,
  };
}

describe("LiveRefGroup reference context", () => {
  it("expands an Unlinked References breadcrumb while retaining excerpts before expansion (GH #526)", async () => {
    const { page, result } = hierarchy();
    loadSingle(page);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([{ page: page.name, kind: page.kind, blocks: [result] }]);
    const { root, dispose } = mount(() => <UnlinkedReferences name="Unlinked context target" />);
    try {
      root.querySelector<HTMLElement>(".references-header")!.click();
      await expect.poll(() => root.textContent).toContain("Root");
      expect(root.querySelector(".reference-show-full")?.textContent).toBe("Show full block");
      const crumb = [...root.querySelectorAll<HTMLElement>(".ref-crumb")].find((c) => c.textContent === "Five");
      expect(crumb).toBeDefined();
      crumb!.click();
      await expect.poll(() => root.querySelector(".ls-block")?.getAttribute("data-block-id")).toBe("ancestor-5");
      expect(root.textContent).toContain("Root");
    } finally {
      dispose();
    }
  });

  it.each(["ref", "query"] as const)("expands ancestors per parent group on %s surfaces, resets on unmount, and Shift-click parks context (GH #526)", async (surface) => {
    const leaf = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
    const first = leaf("hit-a", "First hit [[Target]]");
    const second = leaf("hit-b", "Second hit [[Target]]");
    const page: PageDto = {
      name: "Context", title: "Context", kind: "page", pre_block: null,
      blocks: [{ id: "outer", raw: "Outer context", collapsed: true, children: [
        { id: "parent-a", raw: "Parent A", collapsed: true, children: [first, leaf("sibling", "Nonmatching sibling")] },
        { id: "parent-b", raw: "Parent B", collapsed: false, children: [second] },
      ] }],
    };
    loadSingle(page);
    const save = vi.spyOn(backend(), "savePages");
    const renderGroup = () => <LiveRefGroup page={page.name} kind="page" blocks={[first, second]} surface={surface} showBreadcrumb eager />;
    let view = mount(renderGroup);
    const clickCrumb = (text: string, shiftKey = false) => {
      const crumb = [...view.root.querySelectorAll<HTMLElement>(".ref-crumb")].find((c) => c.textContent === text);
      expect(crumb).toBeDefined();
      crumb!.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey }));
    };
    try {
      await expect.poll(() => view.root.textContent).toContain("First hit");
      const currentRoute = route();
      clickCrumb("Parent A", true);
      expect(rightSidebar()[0]).toMatchObject({ kind: "block", uuid: "parent-a", page: page.name });
      expect(view.root.textContent).not.toContain("Nonmatching sibling");
      clickCrumb("Parent A");
      await expect.poll(() => view.root.textContent).toContain("Nonmatching sibling");
      expect([...view.root.querySelectorAll(".ls-block")].map((b) => b.getAttribute("data-block-id")))
        .toEqual(["parent-a", "hit-a", "sibling", "hit-b"]);
      expect(doc.byId["parent-a"].collapsed).toBe(true);
      clickCrumb("Outer context");
      await expect.poll(() => view.root.querySelector('[data-block-id="outer"]')).not.toBeNull();
      // The expanded root is open; its collapsed descendant stays folded.
      expect(view.root.textContent).not.toContain("Nonmatching sibling");
      expect(view.root.querySelector('[data-block-id="parent-a"]')).not.toBeNull();
      expect(doc.byId["outer"].collapsed).toBe(true);
      expect(route()).toEqual(currentRoute);
      expect(save).not.toHaveBeenCalled();
      view.dispose();
      view = mount(renderGroup);
      await expect.poll(() => view.root.textContent).toContain("First hit");
      expect(view.root.querySelector('[data-block-id="parent-a"]')).toBeNull();
      expect(view.root.querySelector('[data-block-id="outer"]')).toBeNull();
    } finally {
      view.dispose();
    }
  });

  it("plain-clicks a hydrated ancestor without navigating or opening a sidebar (GH #526)", async () => {
    const { page, result } = hierarchy();
    loadSingle(page);
    const { root, dispose } = mount(() => (
      <LiveRefGroup page={page.name} kind={page.kind} blocks={[result]} surface="ref" showBreadcrumb eager />
    ));
    try {
      await expect.poll(() => root.textContent).toContain("Root");
      const before = route();
      [...root.querySelectorAll<HTMLElement>(".ref-crumb")].find((c) => c.textContent === "Five")!.click();
      await expect.poll(() => root.querySelector(".ls-block")?.getAttribute("data-block-id")).toBe("ancestor-5");
      expect(root.textContent).toContain("Root");
      expect(route()).toEqual(before);
      expect(rightSidebar()).toEqual([]);
    } finally {
      dispose();
    }
  });

  it("drops a source page read after its group unmounts", async () => {
    const { page, result } = hierarchy();
    let finish!: (value: PageRead) => void;
    const read = vi.spyOn(backend(), "getPage").mockImplementation(() =>
      new Promise((resolve) => { finish = resolve; }));
    const { dispose } = mount(() => (
      <LiveRefGroup page={page.name} kind={page.kind} blocks={[result]} surface="ref" />
    ));
    await expect.poll(() => read.mock.calls.length).toBe(1);
    dispose();
    finish({ ...page, id: "pages/source.md" });
    await Promise.resolve();
    await Promise.resolve();
    expect(pageByName(page.name), "I-20: a retired LiveRefGroup cannot populate the shared page store").toBeUndefined();
  });

  it("bounds a hit breadcrumb to the final three ancestors and marks omitted context", async () => {
    const { page, result } = hierarchy();
    const topLevel: BlockDto = {
      id: "top-level-hit",
      raw: "Top-level [[Target]]",
      collapsed: false,
      children: [],
      breadcrumb: [],
    };
    page.blocks.push(topLevel);
    loadSingle(page);
    const { root, dispose } = mount(() => (
      <LiveRefGroup page={page.name} kind={page.kind} blocks={[result, topLevel]} surface="ref" showBreadcrumb />
    ));

    try {
      await expect.poll(() => root.querySelector(".ref-breadcrumb")?.textContent?.replace(/\s+/g, "").trim())
        .toBe("…›Three›Four›Five");
      expect(root.querySelectorAll(".ref-breadcrumb")).toHaveLength(1);
      expect(root.textContent).toContain("Top-level");
    } finally {
      dispose();
    }
  });

  it("renders one breadcrumb per shared parent and gathers siblings under it (OG group-by :block/parent)", async () => {
    const leaf = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
    const s1 = leaf("sib-1", "Sibling one [[Target]]");
    const s2 = leaf("sib-2", "Sibling two [[Target]]");
    const t1 = leaf("other-1", "Other [[Target]]");
    const page: PageDto = {
      name: "Grouped",
      title: "Grouped",
      kind: "page",
      pre_block: null,
      blocks: [
        { id: "parent-a", raw: "Parent A", collapsed: false, children: [s1, s2] },
        { id: "parent-b", raw: "Parent B", collapsed: false, children: [t1] },
      ],
    };
    loadSingle(page);
    const hit = (b: BlockDto, crumb: string) => ({ ...b, breadcrumb: [crumb] });
    // Interleaved on purpose: OG groups by parent, so the siblings join up.
    const blocks = [hit(s1, "Parent A"), hit(t1, "Parent B"), hit(s2, "Parent A")];
    const { root, dispose } = mount(() => (
      <LiveRefGroup page={page.name} kind={page.kind} blocks={blocks} surface="query" showBreadcrumb />
    ));
    try {
      await expect.poll(() => root.querySelectorAll(".ref-breadcrumb").length).toBe(2);
      const crumbs = [...root.querySelectorAll(".ref-breadcrumb")].map((c) => c.textContent?.replace(/\s+/g, "").trim());
      expect(crumbs).toEqual(["ParentA", "ParentB"]);
      const text = root.textContent ?? "";
      expect(text.indexOf("Sibling one")).toBeLessThan(text.indexOf("Sibling two"));
      expect(text.indexOf("Sibling two")).toBeLessThan(text.indexOf("Other"));
    } finally {
      dispose();
    }
  });

  it("defaults the first descendant branch closed, keeps toggles view-local, and survives result-object refresh", async () => {
    const { page, result } = hierarchy();
    loadSingle(page);
    const [membership, setMembership] = createSignal([result]);
    const { root, dispose } = mount(() => (
      <LiveRefGroup page={page.name} kind={page.kind} blocks={membership()} surface="ref" />
    ));

    try {
      await expect.poll(() => root.textContent).toContain("Child body");
      expect(root.textContent).not.toContain("Grandchild body");

      const childToggle = root.querySelector<HTMLElement>(
        '[data-block-id="ref-child"] > .block-main .collapse-toggle.has-children',
      );
      expect(childToggle).not.toBeNull();
      childToggle!.click();
      await expect.poll(() => root.textContent).toContain("Grandchild body");

      setMembership([{ ...result, breadcrumb: ["Changed object identity"] }]);
      await expect.poll(() => root.textContent).toContain("Grandchild body");

      setDoc("byId", "new-child", {
        id: "new-child",
        raw: "New child from a reactive source edit",
        collapsed: false,
        parent: "ref-child",
        page: page.name,
        children: [],
      });
      setDoc("byId", "ref-child", "children", ["ref-grandchild", "new-child"]);
      await expect.poll(() => root.textContent).toContain("New child from a reactive source edit");
      expect(doc.byId["ref-child"].collapsed).toBe(false);
      expect(doc.byId["ref-child"].raw).not.toContain("collapsed::");
    } finally {
      dispose();
    }
  });

  it("retains source collapse for the displayed hit root", async () => {
    const { page, result, sourceHit } = hierarchy();
    sourceHit.collapsed = true;
    result.collapsed = true;
    loadSingle(page);
    const { root, dispose } = mount(() => (
      <LiveRefGroup page={page.name} kind={page.kind} blocks={[result]} surface="ref" />
    ));

    try {
      await expect.poll(() => root.textContent).toContain("Root");
      expect(root.textContent).not.toContain("Child body");
      expect(doc.byId["ref-root"].collapsed).toBe(true);
    } finally {
      dispose();
    }
  });
});

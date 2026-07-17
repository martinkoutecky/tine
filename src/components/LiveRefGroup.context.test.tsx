import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { doc, loadSingle, resetStore, setDoc } from "../store";
import type { BlockDto, PageDto } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
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

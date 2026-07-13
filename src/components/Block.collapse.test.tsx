import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { doc, resetStore, setDoc, undo, type FeedPage, type Node as StoreNode } from "../store";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function node(
  id: string,
  raw: string,
  parent: string | null,
  children: string[] = [],
  collapsed = false
): StoreNode {
  return { id, raw, collapsed, parent, page: "Jul 9th, 2026", children };
}

describe("collapsed heading blocks", () => {
  it("renders the parent heading and hides only its children", () => {
    const parent = "parent";
    const child = "child";
    const page: FeedPage = {
      name: "Jul 9th, 2026",
      kind: "journal",
      title: "Jul 9th, 2026",
      preBlock: null,
      roots: [parent],
      format: "md",
      readOnly: false,
      guide: false,
    };
    setDoc({
      byId: {
        [parent]: node(
          parent,
          "# Park Ji Hyun Confirmed To Reunite With Song Joong Ki In New Romance Drama\ncollapsed:: true",
          null,
          [child],
          true
        ),
        [child]: node(child, "Child article content", parent),
      },
      pages: [page],
      feed: [page.name],
      loaded: true,
    });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <Block id={parent} />, host);
    try {
      const parentEl = host.querySelector(`[data-block-id="${parent}"]`);
      expect(parentEl?.textContent).toContain(
        "Park Ji Hyun Confirmed To Reunite With Song Joong Ki In New Romance Drama"
      );
      expect(parentEl?.textContent).not.toContain("collapsed");
      expect(host.querySelector(`[data-block-id="${child}"]`)).toBeNull();
    } finally {
      dispose();
    }
  });
});

describe("outline guide descendant collapse", () => {
  it("collapses or expands every collapsible descendant without folding the guide parent", () => {
    const page: FeedPage = {
      name: "Jul 9th, 2026",
      kind: "journal",
      title: "Jul 9th, 2026",
      preBlock: null,
      roots: ["root"],
      format: "md",
      readOnly: false,
      guide: false,
    };
    setDoc({
      byId: {
        root: node("root", "Root", null, ["child", "leaf"]),
        child: node("child", "Child", "root", ["grandchild"]),
        grandchild: node("grandchild", "Grandchild", "child", ["great"]),
        great: node("great", "Great", "grandchild"),
        leaf: node("leaf", "Leaf", "root"),
      },
      pages: [page],
      feed: [page.name],
      loaded: true,
    });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <Block id="root" />, host);
    try {
      const guide = host.querySelector<HTMLButtonElement>(
        `[data-block-id="root"] > .block-children-container > button.block-children-left-border`
      );
      expect(guide).not.toBeNull();
      expect(guide?.getAttribute("aria-label")).toBe("Collapse all descendants");
      guide?.click();

      expect(doc.byId.root.collapsed).toBe(false);
      expect(doc.byId.child.collapsed).toBe(true);
      expect(doc.byId.grandchild.collapsed).toBe(true);
      expect(doc.byId.leaf.collapsed).toBe(false);
      expect(doc.byId.child.raw).toContain("collapsed:: true");

      undo();
      expect(doc.byId.child.collapsed).toBe(false);
      expect(doc.byId.grandchild.collapsed).toBe(false);
      expect(doc.byId.child.raw).not.toContain("collapsed:: true");

      guide?.click();
      guide?.click();
      expect(doc.byId.child.collapsed).toBe(false);
      expect(doc.byId.grandchild.collapsed).toBe(false);
    } finally {
      dispose();
    }
  });
});

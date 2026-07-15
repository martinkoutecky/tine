import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { Show, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import {
  doc,
  pageByName,
  readPageProperty,
  resetStore,
  setDoc,
  undo,
  type FeedPage,
  type Node as StoreNode,
} from "../store";
import { editingId, endEdit, startEditing } from "../editorController";
import { journalTitle } from "../journal";
import type { RefGroup } from "../types";
import { TagPageTable, TagTableToggle } from "./Page";
import { PageView } from "./Page";
import { focusBlock, mainPaneRouter, resetTabsToJournals } from "../router";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  endEdit("blur");
  resetStore();
  document.body.innerHTML = "";
  resetTabsToJournals();
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function page(name: string, kind: "page" | "journal", roots: string[], preBlock: string | null = null): FeedPage {
  return { name, kind, title: name, preBlock, roots, format: "md", readOnly: false, guide: false };
}

function node(id: string, raw: string, pageName: string, parent: string | null = null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: pageName, children };
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

describe("tag-page table", () => {
  it("toggles a query-sourced table and adds new rows to today's journal", async () => {
    const todayName = journalTitle(new Date());
    setDoc({
      byId: {
        existing: node("existing", "existing", todayName),
        row: node("row", "TODO Tagged row #Tag\nowner:: Martin", "Source"),
      },
      pages: [
        page("Tag", "page", []),
        page("Source", "page", ["row"]),
        page(todayName, "journal", ["existing"]),
      ],
      feed: ["Tag"],
      loaded: true,
    });
    const groups: RefGroup[] = [
      {
        page: "Source",
        kind: "page",
        blocks: [
          {
            id: "row",
            raw: doc.byId.row.raw,
            collapsed: false,
            children: [],
            marker: "TODO",
            tags: ["Tag"],
            properties: [["owner", "Martin"]],
          },
        ],
      },
    ];
    vi.spyOn(backend(), "runQuery").mockResolvedValue(groups);
    vi.spyOn(backend(), "savePage").mockResolvedValue("rev1");

    const tagPage = pageByName("Tag")!;
    const { root, dispose } = mount(() => (
      <>
        <TagTableToggle page={tagPage} />
        <Show when={readPageProperty("Tag", "tine.tag-table") === "true"}>
          <TagPageTable pageName="Tag" />
        </Show>
      </>
    ));

    await tick();
    const toggle = root.querySelector(".tag-table-toggle") as HTMLButtonElement | null;
    expect(toggle).not.toBeNull();
    toggle!.click();
    expect(readPageProperty("Tag", "tine.tag-table")).toBe("true");

    await tick();
    expect(root.textContent).toContain("Tagged row");
    expect(root.textContent).toContain("Martin");

    (root.querySelector(".sheet-add-row-ghost") as HTMLButtonElement).click();
    await tick();
    await tick();

    const today = pageByName(todayName)!;
    const newId = today.roots[today.roots.length - 1];
    expect(doc.byId[newId].raw).toMatch(/^#Tag\s*$/);
    expect(editingId()).toBe(newId);

    dispose();
  });
});

describe("zoomed block view", () => {
  it("reveals a collapsed root's children without changing its stored collapse state", async () => {
    const parent = "11111111-1111-4111-8111-111111111111";
    const child = "22222222-2222-4222-8222-222222222222";
    const dto = {
      name: "Outline",
      kind: "page" as const,
      title: "Outline",
      pre_block: null,
      blocks: [{
        id: parent,
        raw: "Collapsed section\ncollapsed:: true\nid:: 11111111-1111-4111-8111-111111111111",
        collapsed: true,
        children: [{ id: child, raw: "Hidden child", collapsed: false, children: [] }],
      }],
    };
    setDoc({
      byId: {
        [parent]: { ...node(parent, dto.blocks[0].raw, dto.name, null, [child]), collapsed: true },
        [child]: node(child, "Hidden child", dto.name, parent),
      },
      pages: [page(dto.name, "page", [parent])],
      feed: [dto.name],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    focusBlock(parent);

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      expect(root.querySelector(`[data-block-id="${child}"]`)).not.toBeNull();
      expect(doc.byId[parent].collapsed).toBe(true);
    } finally {
      dispose();
    }
  });

  it("Enter at a collapsed zoom root creates and focuses a rendered child, not an outside sibling", async () => {
    const parent = "11111111-1111-4111-8111-111111111111";
    const oldChild = "22222222-2222-4222-8222-222222222222";
    const outside = "33333333-3333-4333-8333-333333333333";
    const dto = {
      name: "Outline",
      kind: "page" as const,
      title: "Outline",
      pre_block: null,
      blocks: [
        { id: parent, raw: "Root\ncollapsed:: true", collapsed: true, children: [{ id: oldChild, raw: "Old", collapsed: false, children: [] }] },
        { id: outside, raw: "Outside", collapsed: false, children: [] },
      ],
    };
    setDoc({
      byId: {
        [parent]: { ...node(parent, dto.blocks[0].raw, dto.name, null, [oldChild]), collapsed: true },
        [oldChild]: node(oldChild, "Old", dto.name, parent),
        [outside]: node(outside, "Outside", dto.name),
      },
      pages: [page(dto.name, "page", [parent, outside])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    focusBlock(parent);
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      startEditing(parent, 0);
      await tick();
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      textarea.selectionStart = 0;
      textarea.selectionEnd = 0;
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      await tick();
      const created = doc.byId[parent].children[0];
      expect(created).not.toBe(oldChild);
      expect(editingId()).toBe(created);
      expect(root.querySelector(`[data-block-id="${created}"] textarea`)).not.toBeNull();
      expect(doc.pages[0].roots).toEqual([parent, outside]);
      expect(doc.byId[parent].collapsed).toBe(true);
    } finally {
      dispose();
    }
  });
});

describe("trailing page block target", () => {
  it("creates one focused root, accepts immediate input, then reuses its empty trailing leaf", async () => {
    const dto = {
      name: "Continue",
      kind: "page" as const,
      title: "Continue",
      pre_block: null,
      blocks: [{ id: "last", raw: "Last text", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { last: node("last", "Last text", "Continue") },
      pages: [page("Continue", "page", ["last"])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage("Continue", "page");
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      const target = root.querySelector(".page-trailing-block-target") as HTMLButtonElement;
      target.click();
      await tick();
      expect(doc.pages[0].roots).toHaveLength(2);
      const created = doc.pages[0].roots[1];
      expect(editingId()).toBe(created);
      const textarea = root.querySelector(`[data-block-id="${created}"] textarea`) as HTMLTextAreaElement | null;
      expect(textarea).not.toBeNull();
      expect(document.activeElement).toBe(textarea);
      expect(textarea?.selectionStart).toBe(0);
      expect(textarea?.selectionEnd).toBe(0);
      textarea!.value = "typed immediately";
      textarea!.dispatchEvent(new Event("input", { bubbles: true }));
      expect(doc.byId[created].raw).toBe("typed immediately");
      textarea!.value = "";
      textarea!.dispatchEvent(new Event("input", { bubbles: true }));
      endEdit("blur");
      target.click();
      await tick();
      expect(doc.pages[0].roots).toEqual(["last", created]);
      expect(editingId()).toBe(created);
    } finally { dispose(); }
  });

  it("keeps creation as one structural Undo entry when no text edit intervenes", async () => {
    const dto = {
      name: "Undo tail", kind: "page" as const, title: "Undo tail", pre_block: null,
      blocks: [{ id: "last", raw: "Last text", collapsed: false, children: [] }],
    };
    setDoc({ byId: { last: node("last", "Last text", dto.name) }, pages: [page(dto.name, "page", ["last"])], feed: [], loaded: true });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage(dto.name, "page");
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      (root.querySelector(".page-trailing-block-target") as HTMLButtonElement).click();
      await tick();
      endEdit("blur");
      undo();
      expect(doc.pages[0].roots).toEqual(["last"]);
    } finally { dispose(); }
  });

  it("adds a zoom-root child and hides the target on read-only pages", async () => {
    const dto = {
      name: "Zoom",
      kind: "page" as const,
      title: "Zoom",
      pre_block: null,
      blocks: [{ id: "zoom", raw: "Root", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { zoom: node("zoom", "Root", "Zoom") },
      pages: [page("Zoom", "page", ["zoom"])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    focusBlock("zoom");
    const mounted = mount(() => <PageView />);
    await tick(); await tick();
    (mounted.root.querySelector(".page-trailing-block-target") as HTMLButtonElement).click();
    await tick();
    expect(doc.byId.zoom.children).toHaveLength(1);
    expect(doc.byId[doc.byId.zoom.children[0]].parent).toBe("zoom");
    mounted.dispose();

    setDoc("pages", 0, "readOnly", true);
    const readonly = mount(() => <PageView />);
    await tick();
    expect(readonly.root.querySelector(".page-trailing-block-target")).toBeNull();
    readonly.dispose();
  });
});

describe("page route loading", () => {
  it("ignores an obsolete load failure after a newer route has loaded", async () => {
    const fastId = "11111111-1111-4111-8111-111111111111";
    const fast = {
      name: "Fast page",
      kind: "page" as const,
      title: "Fast page",
      pre_block: null,
      blocks: [{ id: fastId, raw: "new route content", collapsed: false, children: [] }],
    };
    vi.spyOn(backend(), "journalsDesc").mockResolvedValue([]);
    let rejectSlow!: (reason: Error) => void;
    let resolveFast!: (value: typeof fast) => void;
    vi.spyOn(backend(), "getPage").mockImplementation((name) => {
      if (name === "Slow page") {
        return new Promise((_, reject) => { rejectSlow = reject; });
      }
      return new Promise((resolve) => { resolveFast = resolve as (value: typeof fast) => void; });
    });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      mainPaneRouter.openPage("Slow page", "page", { inPlace: true });
      await tick();
      mainPaneRouter.openPage(fast.name, "page", { inPlace: true });
      await tick();
      resolveFast(fast);
      await tick();
      await tick();
      expect(root.textContent).toContain("new route content");

      rejectSlow(new Error("obsolete slow-page failure"));
      await tick();
      await tick();
      expect(root.textContent).toContain("new route content");
      expect(root.textContent).not.toContain("obsolete slow-page failure");
    } finally {
      dispose();
    }
  });

  it("does not select a collapsed final root's hidden empty descendant", async () => {
    const dto = {
      name: "Collapsed tail",
      kind: "page" as const,
      title: "Collapsed tail",
      pre_block: null,
      blocks: [{
        id: "lead",
        raw: "Lead",
        collapsed: false,
        children: [],
      }, {
        id: "parent",
        raw: "collapsed:: true\nid:: parent",
        collapsed: true,
        children: [{ id: "hidden", raw: "", collapsed: false, children: [] }],
      }],
    };
    setDoc({
      byId: {
        lead: node("lead", "Lead", dto.name),
        parent: { ...node("parent", dto.blocks[1].raw, dto.name, null, ["hidden"]), collapsed: true },
        hidden: node("hidden", "", dto.name, "parent"),
      },
      pages: [page(dto.name, "page", ["lead", "parent"])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage(dto.name, "page");
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      (root.querySelector(".page-trailing-block-target") as HTMLButtonElement).click();
      await tick();
      expect(editingId()).not.toBe("hidden");
      expect(doc.pages[0].roots).toHaveLength(3);
      const created = doc.pages[0].roots[2];
      expect(root.querySelector(`[data-block-id="${created}"] textarea`)).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("does not select storage children of a blank-looking opaque Sheet tail", async () => {
    const dto = {
      name: "Opaque tail",
      kind: "page" as const,
      title: "Opaque tail",
      pre_block: null,
      blocks: [{
        id: "lead",
        raw: "Lead",
        collapsed: false,
        children: [],
      }, {
        id: "grid",
        raw: "tine.view:: grid",
        collapsed: false,
        children: [{ id: "storage", raw: "", collapsed: false, children: [] }],
      }],
    };
    setDoc({
      byId: {
        lead: node("lead", "Lead", dto.name),
        grid: node("grid", dto.blocks[1].raw, dto.name, null, ["storage"]),
        storage: node("storage", "", dto.name, "grid"),
      },
      pages: [page(dto.name, "page", ["lead", "grid"])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage(dto.name, "page");
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      (root.querySelector(".page-trailing-block-target") as HTMLButtonElement).click();
      await tick();
      expect(editingId()).not.toBe("storage");
      expect(doc.pages[0].roots).toHaveLength(3);
      const created = doc.pages[0].roots[2];
      expect(root.querySelector(`[data-block-id="${created}"] textarea`)).not.toBeNull();
    } finally {
      dispose();
    }
  });
});

describe("page properties", () => {
  it("keeps a first-bullet alias editor mounted while the property is being typed (GH #62)", async () => {
    const propsId = "11111111-1111-4111-8111-111111111111";
    const dto = {
      name: "Books",
      kind: "page" as const,
      title: "Books",
      pre_block: null,
      blocks: [{ id: propsId, raw: "", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [propsId]: node(propsId, "", dto.name) },
      pages: [page(dto.name, "page", [propsId])],
      feed: [dto.name],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      startEditing(propsId, 0);
      await tick();
      setDoc("byId", propsId, "raw", "alias::");
      await tick();
      expect(editingId()).toBe(propsId);
      expect(root.querySelector(`[data-block-id="${propsId}"] textarea`)).not.toBeNull();

      setDoc("byId", propsId, "raw", "alias:: book");
      await tick();
      expect(root.querySelector(`[data-block-id="${propsId}"] textarea`)).not.toBeNull();

      endEdit("blur");
      await tick();
      expect(root.querySelector(`[data-block-id="${propsId}"]`)).toBeNull();
      expect(root.querySelector(".page-aliases")?.textContent).toContain("book");
    } finally {
      dispose();
    }
  });

  it("renders a properties-only first block as page properties (GH #86)", async () => {
    const propsId = "11111111-1111-4111-8111-111111111111";
    const bodyId = "22222222-2222-4222-8222-222222222222";
    const dto = {
      name: "Books",
      kind: "page" as const,
      title: "Books",
      pre_block: null,
      blocks: [
        { id: propsId, raw: "alias:: book\ntags:: blah", collapsed: false, children: [] },
        { id: bodyId, raw: "Reading list", collapsed: false, children: [] },
      ],
    };
    setDoc({
      byId: {
        [propsId]: node(propsId, dto.blocks[0].raw, dto.name),
        [bodyId]: node(bodyId, dto.blocks[1].raw, dto.name),
      },
      pages: [page(dto.name, "page", [propsId, bodyId])],
      feed: [dto.name],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      expect(root.querySelector(".page-aliases")?.textContent).toContain("book");
      expect(root.querySelector(".page-properties")?.textContent).toContain("blah");
      expect(root.querySelector(`[data-block-id="${propsId}"]`)).toBeNull();
      expect(root.querySelector(`[data-block-id="${bodyId}"]`)).not.toBeNull();
    } finally {
      dispose();
    }
  });

  it("links bare alias and tag values but leaves ordinary bare properties as text (GH #139)", async () => {
    const propsId = "11111111-1111-4111-8111-111111111111";
    const bodyId = "22222222-2222-4222-8222-222222222222";
    const raw = [
      "aliases:: Book shelf，Reading",
      "tags:: books, [[Knowledge work]]",
      "owner:: Martin",
      "reviewer:: [[Jane Doe]]",
      'status:: "Draft, Private"',
    ].join("\n");
    const dto = {
      name: "Books",
      kind: "page" as const,
      title: "Books",
      pre_block: null,
      blocks: [
        { id: propsId, raw, collapsed: false, children: [] },
        { id: bodyId, raw: "Reading list", collapsed: false, children: [] },
      ],
    };
    setDoc({
      byId: {
        [propsId]: node(propsId, raw, dto.name),
        [bodyId]: node(bodyId, dto.blocks[1].raw, dto.name),
      },
      pages: [page(dto.name, "page", [propsId, bodyId])], feed: [dto.name], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      const rows = [...root.querySelectorAll<HTMLElement>(".prop-row")];
      const row = (key: string) => rows.find((candidate) => candidate.querySelector(".prop-key")?.textContent === key)!;
      expect([...row("tags").querySelectorAll(".page-ref")].map((link) => link.textContent)).toEqual(["books", "Knowledge work"]);
      expect([...row("aliases").querySelectorAll(".page-ref")].map((link) => link.textContent)).toEqual(["Book shelf", "Reading"]);
      expect(row("owner").querySelector(".page-ref")).toBeNull();
      // Explicit custom-property refs keep the app's ordinary dimmed [[bracket]]
      // styling; only the newly inferred bare built-ins need a plain label.
      expect([...row("reviewer").querySelectorAll(".page-ref")].map((link) => link.textContent)).toEqual(["[[Jane Doe]]"]);
      expect(row("status").querySelector(".page-ref")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("keeps an editable blank body after hiding an only properties block", async () => {
    const propsId = "11111111-1111-4111-8111-111111111111";
    const dto = {
      name: "Only properties",
      kind: "page" as const,
      title: "Only properties",
      pre_block: null,
      blocks: [{ id: propsId, raw: "alias:: property-only", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [propsId]: node(propsId, dto.blocks[0].raw, dto.name) },
      pages: [page(dto.name, "page", [propsId])], feed: [dto.name], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      const visibleRoots = pageByName(dto.name)!.roots.filter((id) => id !== propsId);
      expect(visibleRoots).toHaveLength(1);
      expect(doc.byId[visibleRoots[0]].raw).toBe("");
      expect(root.querySelector(`[data-block-id="${visibleRoots[0]}"]`)).not.toBeNull();
    } finally {
      dispose();
    }
  });
});

describe("Markdown preamble content", () => {
  it("renders text before the first bullet and promotes it only when edited (GH #85)", async () => {
    const bodyId = "22222222-2222-4222-8222-222222222222";
    const dto = {
      name: "Imported",
      kind: "page" as const,
      title: "Imported",
      pre_block: "Intro before the outline",
      blocks: [{ id: bodyId, raw: "First marked block", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { [bodyId]: node(bodyId, dto.blocks[0].raw, dto.name) },
      pages: [page(dto.name, "page", [bodyId], dto.pre_block)],
      feed: [dto.name],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      const preamble = root.querySelector(".preamble-block .block-content-wrapper") as HTMLElement | null;
      expect(preamble?.textContent).toContain("Intro before the outline");
      expect(pageByName(dto.name)?.preBlock).toBe(dto.pre_block);

      preamble!.click();
      await tick();
      const promoted = pageByName(dto.name)!.roots[0];
      expect(doc.byId[promoted].raw).toBe("Intro before the outline");
      expect(pageByName(dto.name)?.preBlock).toBeNull();
      expect(editingId()).toBe(promoted);
      expect(root.querySelector(`[data-block-id="${promoted}"] textarea`)).not.toBeNull();
    } finally {
      dispose();
    }
  });
});

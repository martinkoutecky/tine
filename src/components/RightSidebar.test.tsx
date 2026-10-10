import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { editingId, endEdit } from "../editorController";
import { initParser } from "../render/parse";
import { blockRef, blockPositionRef, ensureBlockId, isDirty, pageByName, resetStore } from "../document";
import { bindTestHost } from "../document/host/wiring.test.support";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { PageDto, PageRead } from "../types";
import { applySidebarSession, openBlockInSidebar, rightSidebar, setRightSidebar } from "../ui";
import { RightSidebar } from "./RightSidebar";
import { route } from "../router";

const page: PageDto = {
  name: "Sidebar test",
  kind: "page",
  title: "Sidebar test",
  pre_block: null,
  blocks: [
    {
      id: "sidebar-root",
      raw: "Editable sidebar text",
      collapsed: false,
      children: [{ id: "sidebar-child", raw: "Sidebar child", collapsed: false, children: [] }],
    },
    { id: "sidebar-second", raw: "Second block", collapsed: false, children: [] },
  ],
};

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  endEdit("page-navigation");
  setRightSidebar([]);
  applySidebarSession({ right: false, items: [] });
  resetStore();
  document.body.innerHTML = "";
  localStorage.clear();
  vi.restoreAllMocks();
});

function mount(items = [
  { kind: "page" as const, name: page.name, pageKind: "page" as const },
  { kind: "block" as const, uuid: "sidebar-second", page: page.name, pageKind: "page" as const },
]) {
  loadSingle(page);
  applySidebarSession({ right: true, items });
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <RightSidebar />, root);
  return { root, dispose };
}

describe("right sidebar collection disclosures", () => {
  it("names the source page above an opened block (GH #640)", async () => {
    const { root, dispose } = mount([
      { kind: "block", uuid: "sidebar-second", page: page.name, pageKind: "page" },
    ]);
    try {
      await expect.poll(() => root.querySelector(".rs-item-title")?.textContent).toBe(page.name);
      expect(root.textContent).toContain("Second block");
      root.querySelector<HTMLElement>(".rs-item-title")!.click();
      expect(route()).toMatchObject({ kind: "page", name: page.name });
    } finally {
      dispose();
    }
  });

  it("keeps the page label and missing-block notice for a deleted sidebar target (GH #640)", async () => {
    const { root, dispose } = mount([
      { kind: "block", uuid: "deleted-block", page: page.name, pageKind: "page" },
    ]);
    try {
      await expect.poll(() => root.textContent).toContain("This block is no longer available.");
      expect(root.querySelector(".rs-item-title")?.textContent).toBe(page.name);
    } finally {
      dispose();
    }
  });

  it("opening a fresh block in the sidebar writes nothing and stores its position, not an id", async () => {
    await bindTestHost();
    const save = vi.spyOn(backend(), "pageSubmit");
    loadSingle({
      ...page,
      blocks: [{ id: "bfresh-sidebar", raw: "Fresh sidebar target", collapsed: false, children: [] }],
    });

    openBlockInSidebar(blockPositionRef(blockRef("bfresh-sidebar")));

    expect(rightSidebar()[0]).toMatchObject({ kind: "block", page: page.name, pageKind: "page", blockPos: [0] });
    expect(doc.byId["bfresh-sidebar"].raw).toBe("Fresh sidebar target");
    expect(save).not.toHaveBeenCalled();
    expect(isDirty(page.name)).toBe(false);
  });

  it("Copy block ref on a sidebar block still stamps its durable UUID", async () => {
    const uuid = "12345678-1234-4234-8234-123456789abc";
    vi.spyOn(crypto, "randomUUID").mockReturnValue(uuid);
    await bindTestHost();
    loadSingle({
      ...page,
      blocks: [{ id: "bfresh-sidebar", raw: "Fresh sidebar target", collapsed: false, children: [] }],
    });
    expect(await ensureBlockId("bfresh-sidebar")).toBe(uuid);
    expect(doc.byId["bfresh-sidebar"].raw).toBe(`Fresh sidebar target\nid:: ${uuid}`);
  });

  it("resolves a durable Org sidebar UUID to its transient live store node", async () => {
    const uuid = "12345678-1234-4234-8234-123456789abc";
    const transient = "bfresh-org-sidebar";
    const orgPage: PageRead = {
      name: "2026-07-22",
      kind: "journal",
      title: "Wednesday, 22 July 2026",
      pre_block: null,
      format: "org",
      id: "journals/2026_07_22.org",
      blocks: [{
        id: transient,
        raw: `Fresh Org target\n:PROPERTIES:\n:id: ${uuid}\n:END:`,
        collapsed: false,
        children: [],
      }],
    };
    loadSingle(orgPage);
    applySidebarSession({
      right: true,
      items: [{
        kind: "block",
        uuid,
        page: orgPage.name,
        pageKind: "journal",
        path: orgPage.id,
      }],
    });
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <RightSidebar />, root);

    try {
      await vi.waitFor(() => {
        expect(root.querySelector(`[data-block-id="${transient}"]`)).not.toBeNull();
        expect(root.textContent).toContain("Fresh Org target");
      });
    } finally {
      dispose();
    }
  });

  it("replaces a same-name loaded page with the sidebar item's exact physical owner", async () => {
    loadSingle({ ...page, id: "pages/Sidebar test.md" });
    applySidebarSession({
      right: true,
      items: [{
        kind: "page",
        name: page.name,
        pageKind: "page",
        path: "pages/duplicates/Sidebar test.md",
      }],
    });
    const exact: PageRead = {
      ...page,
      id: "pages/duplicates/Sidebar test.md",
      blocks: [{ id: "exact-root", raw: "Noncanonical exact content", collapsed: false, children: [] }],
    };
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    const getPageByPath = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(exact);
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <RightSidebar />, root);

    try {
      await vi.waitFor(() => {
        expect(root.textContent).toContain("Noncanonical exact content");
        expect(pageByName(page.name)?.id).toBe("pages/duplicates/Sidebar test.md");
      });
      expect(getPageByPath).toHaveBeenCalledWith("pages/duplicates/Sidebar test.md");
      expect(getPage).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("adopts the canonical page name for a restored mixed-case sidebar item", async () => {
    const canonical = { ...page, name: "page1", title: "page1" };
    applySidebarSession({
      right: true,
      items: [{ kind: "page", name: "Page1", pageKind: "page" }],
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(canonical as PageRead);
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <RightSidebar />, root);

    try {
      await vi.waitFor(() => {
        expect(rightSidebar()[0]).toMatchObject({ kind: "page", name: "page1" });
        expect(root.textContent).toContain("Editable sidebar text");
      });
    } finally {
      dispose();
    }
  });

  it("keeps a real sidebar Block disclosure separate from the sidebar item's disclosure", async () => {
    const { root, dispose } = mount([
      { kind: "page", name: page.name, pageKind: "page" },
    ]);
    try {
      const parentToggle = await vi.waitFor(() => {
        const found = root.querySelector<HTMLElement>(
          '[data-block-id="sidebar-root"] > .block-main .collapse-toggle.has-children'
        );
        expect(found).not.toBeNull();
        expect(root.querySelector('[data-block-id="sidebar-child"] > .block-main .collapse-toggle:not(.has-children)')).not.toBeNull();
        expect(root.querySelectorAll('[data-block-id="sidebar-root"] > .block-main .collapse-toggle.has-children')).toHaveLength(1);
        return found!;
      });
      const itemToggle = root.querySelector<HTMLButtonElement>("[data-right-sidebar-item-toggle]")!;
      expect(itemToggle.getAttribute("aria-expanded")).toBe("true");
      expect(root.querySelector(".rs-item-body")).not.toBeNull();

      parentToggle.click();
      await vi.waitFor(() => expect(root.querySelector('[data-block-id="sidebar-child"]')).toBeNull());
      expect(root.querySelector(".rs-item-body")).not.toBeNull();
      expect(itemToggle.getAttribute("aria-expanded")).toBe("true");

      root.querySelector<HTMLElement>('[data-block-id="sidebar-root"] > .block-main .collapse-toggle.has-children')!.click();
      await vi.waitFor(() => expect(root.querySelector('[data-block-id="sidebar-child"]')).not.toBeNull());
    } finally {
      dispose();
    }
  });

  it("collapses items independently and offers collapse, expand, and close all", async () => {
    const { root, dispose } = mount();
    try {
      const toggles = await vi.waitFor(() => {
        const found = root.querySelectorAll<HTMLButtonElement>("[data-right-sidebar-item-toggle]");
        expect(found).toHaveLength(2);
        return [...found];
      });
      expect(toggles[0].getAttribute("aria-expanded")).toBe("true");
      expect(toggles[0].getAttribute("aria-controls")).toBeTruthy();
      expect(root.querySelectorAll(".rs-item-body")).toHaveLength(2);

      toggles[0].focus();
      toggles[0].click();
      await Promise.resolve();
      expect(root.querySelector("[data-right-sidebar-item-toggle]")?.getAttribute("aria-expanded")).toBe("false");
      expect(document.activeElement).toBe(root.querySelector("[data-right-sidebar-item-toggle]"));
      expect(root.querySelectorAll(".rs-item-body")).toHaveLength(1);

      root.querySelector<HTMLButtonElement>("[data-right-sidebar-actions]")!.click();
      root.querySelector<HTMLButtonElement>('[data-right-sidebar-action="collapse-all"]')!.click();
      expect(root.querySelectorAll(".rs-item-body")).toHaveLength(0);
      expect(rightSidebar().every((item) => item.collapsed)).toBe(true);

      root.querySelector<HTMLButtonElement>("[data-right-sidebar-actions]")!.click();
      root.querySelector<HTMLButtonElement>('[data-right-sidebar-action="expand-all"]')!.click();
      expect(root.querySelectorAll(".rs-item-body")).toHaveLength(2);
      expect(rightSidebar().every((item) => !item.collapsed)).toBe(true);

      root.querySelector<HTMLButtonElement>("[data-right-sidebar-actions]")!.click();
      root.querySelector<HTMLButtonElement>('[data-right-sidebar-action="close-all"]')!.click();
      expect(rightSidebar()).toEqual([]);
      expect(root.textContent).toContain("Nothing open");
    } finally {
      dispose();
    }
  });

  it("commits and exits an active sidebar editor before its body unmounts", async () => {
    const { root, dispose } = mount([
      { kind: "block", uuid: "sidebar-root", page: page.name, pageKind: "page" },
    ]);
    try {
      const content = await vi.waitFor(() => {
        const found = root.querySelector<HTMLElement>(".rs-item-body .block-content-wrapper");
        expect(found).not.toBeNull();
        return found!;
      });
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      content.click();
      const editor = await vi.waitFor(() => {
        const found = root.querySelector<HTMLTextAreaElement>(".rs-item-body textarea.block-editor");
        expect(found).not.toBeNull();
        return found!;
      });
      editor.value = "Committed before collapse";
      editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
      expect(editingId()).toBe("sidebar-root");

      root.querySelector<HTMLButtonElement>("[data-right-sidebar-item-toggle]")!.click();

      expect(root.querySelector(".rs-item-body")).toBeNull();
      expect(editingId()).toBeNull();
      expect(doc.byId["sidebar-root"].raw).toBe("Committed before collapse");
    } finally {
      dispose();
    }
  });
});

// GH #358: a block explicitly parked in the right sidebar is the ROOT of that
// view — its children must render regardless of the source outline's
// persisted collapsed flag (the zoomed view's forceExpandedRoot contract),
// while descendant collapse states stay respected.
describe("right sidebar collapsed-source block (GH #358)", () => {
  const collapsedPage: PageDto = {
    name: "Collapsed source",
    kind: "page",
    title: "Collapsed source",
    pre_block: null,
    blocks: [
      {
        id: "parked-parent",
        raw: "Parked parent",
        collapsed: true, // hidden in the main outline — must not matter here
        children: [
          { id: "parked-child-1", raw: "Child one", collapsed: false, children: [] },
          {
            id: "parked-child-2",
            raw: "Child two",
            collapsed: true, // descendant collapse stays respected
            children: [{ id: "parked-grandchild", raw: "Grandchild", collapsed: false, children: [] }],
          },
        ],
      },
      { id: "unrelated", raw: "Unrelated", collapsed: false, children: [] },
    ],
  };

  function mountParked() {
    loadSingle(collapsedPage);
    applySidebarSession({
      right: true,
      items: [{ kind: "block", uuid: "parked-parent", page: collapsedPage.name, pageKind: "page" }],
    });
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <RightSidebar />, root);
    return { root, dispose };
  }

  it("renders the parked block's children despite its collapsed source state", async () => {
    const { root, dispose } = mountParked();
    try {
      await vi.waitFor(() => {
        expect(root.querySelector('.rs-item-body .ls-block[data-block-id="parked-child-1"]')).not.toBeNull();
        expect(root.querySelector('.rs-item-body .ls-block[data-block-id="parked-child-2"]')).not.toBeNull();
      });
      // …while the descendant's own collapsed state is honored.
      expect(root.querySelector('.rs-item-body .ls-block[data-block-id="parked-grandchild"]')).toBeNull();
      // The main outline's durable state is untouched.
      expect(doc.byId["parked-parent"].collapsed).toBe(true);
      expect(doc.byId["parked-child-2"].collapsed).toBe(true);
    } finally {
      dispose();
    }
  });
});

describe("right sidebar page with nothing to type into (GH #483)", () => {
  // The reporter's exact conditional — "If I add any text on this page in the
  // first place and AFTER that open it in the Sidebar, I can continue to edit"
  // — is the tell. A page shown ONLY in the sidebar never ran the main pane's
  // re-seed effect, so a page with no body rendered an empty box: no bullet, no
  // trailing target, nothing to put a caret in. The two shapes below are the
  // same defect: no roots at all, and a page whose only root is its properties.
  const noBlocks: PageDto = {
    name: "Blank sidebar page", kind: "page", title: "Blank sidebar page", pre_block: null, blocks: [],
  };
  const propertiesOnly: PageDto = {
    name: "Properties only page",
    kind: "page",
    title: "Properties only page",
    pre_block: null,
    blocks: [{ id: "props-only-root", raw: "type:: reference\nstatus:: open", collapsed: false, children: [] }],
  };

  function mountOnly(dto: PageDto) {
    loadSingle(dto);
    applySidebarSession({ right: true, items: [{ kind: "page", name: dto.name, pageKind: "page" }] });
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <RightSidebar />, root);
    return { root, dispose };
  }

  async function expectCaretAvailable(dto: PageDto) {
    const { root, dispose } = mountOnly(dto);
    try {
      const body = await vi.waitFor(() => {
        const found = root.querySelector<HTMLElement>(".rs-item-body");
        expect(found).not.toBeNull();
        return found!;
      });

      // Something editable must exist without the user first visiting the page
      // in the main pane.
      const target = body.querySelector<HTMLButtonElement>(".page-trailing-block-target");
      expect(target, "no trailing block target in the sidebar body").not.toBeNull();

      target!.click();
      await vi.waitFor(() => expect(editingId()).not.toBeNull());
      expect(doc.byId[editingId()!].page).toBe(dto.name);
      expect(doc.byId[editingId()!].raw).toBe("");
    } finally {
      dispose();
    }
  }

  it("offers a caret for a page with no blocks at all", async () => {
    await expectCaretAvailable(noBlocks);
  });

  it("offers a caret for a page whose only block is its properties", async () => {
    await expectCaretAvailable(propertiesOnly);
  });
});

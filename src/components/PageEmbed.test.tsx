import { loadSingle } from "../document/workingSet";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import type { BlockDto } from "../types";
import { EmbedMacro } from "./Macro";
import { KNOBS, setKnob } from "../styleKnobs";
import { layoutPaneIds, paneRouter, resetPaneLayoutToSingle } from "../panes";
import { rightSidebar, setRightSidebar, setRightSidebarOpen } from "../ui";

// GH #439: a whole-page embed names the page it embeds, so it reads as a page and
// not as unexplained rows (OG block.cljs `page-embed` header). The outline-grid
// alignment half of #439 is geometry, checked in headless Chromium by
// scripts/shot-page-embed.mjs (jsdom applies no layout).

beforeAll(async () => {
  await initParser();
});
afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

const leaf = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const pageRead = (name: string, blocks: BlockDto[]) =>
  ({ name, title: name, id: `pages/${name}.md`, kind: "page", format: "md", pre_block: null, blocks }) as never;

async function mountEmbed(body: string): Promise<{ root: HTMLElement; dispose: () => void }> {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <EmbedMacro body={body} />, root);
  await new Promise((resolve) => setTimeout(resolve, 50));
  return { root, dispose };
}

describe("page embed header (GH #439)", () => {
  it("GH #440: an opt-in breadcrumb identifies a block embed's page and ancestors", async () => {
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([
      { page: "Source", kind: "page", blocks: [{ ...leaf("b1", "block row"), breadcrumb: ["Parent", "Child"] }] },
    ] as never);
    vi.spyOn(backend(), "setAppString").mockResolvedValue(undefined);
    const { root, dispose } = await mountEmbed("embed ((b1))");
    try {
      const knob = KNOBS.find(k => k.id === "embed-breadcrumbs");
      expect(knob).toBeDefined();
      expect(document.documentElement.classList.contains("tine-embed-breadcrumbs")).toBe(false);
      setKnob(knob!, true);
      expect(document.documentElement.classList.contains("tine-embed-breadcrumbs")).toBe(true);
      expect(root.querySelector(".embed-source-breadcrumb")?.textContent).toContain("Source");
      expect(root.querySelector(".embed-source-breadcrumb")?.textContent).toContain("Parent");
      expect(root.querySelector(".embed-source-breadcrumb")?.textContent).toContain("Child");
      setKnob(knob!, undefined);
    } finally { dispose(); }
  });
  it("GH #440: with the source loaded, the trail is the live page and nearest three ancestors", async () => {
    let row: BlockDto = leaf("e-leaf", "THE EMBEDDED BLOCK");
    for (const [id, raw] of [["e-4", "Fourth **bold**"], ["e-3", "Third"], ["e-2", "Second"], ["e-1", "First"]] as const)
      row = { id, raw, collapsed: false, children: [row] };
    loadSingle({ name: "Source", title: "Source", kind: "page", pre_block: null, blocks: [row] });
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([
      { page: "Source", kind: "page", blocks: [{ ...leaf("e-leaf", "THE EMBEDDED BLOCK"), breadcrumb: [] }] },
    ] as never);
    vi.spyOn(backend(), "setAppString").mockResolvedValue(undefined);
    const knob = KNOBS.find(k => k.id === "embed-breadcrumbs")!;
    setKnob(knob, true);
    const { root, dispose } = await mountEmbed("embed ((e-leaf))");
    try {
      const trail = root.querySelector(".embed-source-breadcrumb")!;
      expect([...trail.querySelectorAll(".ref-crumb")].map((el) => el.textContent)).toEqual(["…", "Second", "Third", "Fourth bold"]);
      expect(trail.querySelector("strong")?.textContent).toBe("bold");
      expect(trail.textContent).toContain("Source");
      expect(trail.textContent).not.toContain("THE EMBEDDED BLOCK");
      expect(trail.querySelector(".page-ref")?.textContent).toBe("Source");
    } finally { setKnob(knob, undefined); dispose(); }
  });
  describe("GH #779: every ancestor crumb of the embed trail opens that block", () => {
    const snapshot = () => ({ tabs: [{ history: [{ kind: "journals" as const }], pos: 0, pinned: false }], activeIndex: 0 });
    const mountTrail = async () => {
      resetPaneLayoutToSingle(snapshot());
      let row: BlockDto = leaf("e-leaf", "THE EMBEDDED BLOCK");
      for (const [id, raw] of [["e-3", "Third"], ["e-2", "Second"], ["e-1", "First"]] as const)
        row = { id, raw, collapsed: false, children: [row] };
      loadSingle({ name: "Source", title: "Source", kind: "page", pre_block: null, blocks: [row] });
      vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([
        { page: "Source", kind: "page", blocks: [{ ...leaf("e-leaf", "THE EMBEDDED BLOCK"), breadcrumb: [] }] },
      ] as never);
      vi.spyOn(backend(), "setAppString").mockResolvedValue(undefined);
      setKnob(KNOBS.find(k => k.id === "embed-breadcrumbs")!, true);
      const mounted = await mountEmbed("embed ((e-leaf))");
      const crumb = (text: string) => [...mounted.root.querySelectorAll<HTMLElement>(".embed-source-breadcrumb .ref-crumb")]
        .find((el) => el.textContent === text)!;
      return { ...mounted, crumb };
    };
    afterEach(() => {
      setKnob(KNOBS.find(k => k.id === "embed-breadcrumbs")!, undefined);
      setRightSidebar([]);
      setRightSidebarOpen(false);
      resetPaneLayoutToSingle(snapshot());
    });
    it("a plain click goes to that parent block in its page; the embedded block itself is not a crumb", async () => {
      const { root, dispose, crumb } = await mountTrail();
      try {
        expect(crumb("Second").getAttribute("role")).toBe("button");
        expect(root.querySelectorAll(".embed-source-breadcrumb [role=button]")).toHaveLength(3);
        crumb("Second").click();
        // Opens in place in its page (Settings > ref-click-zoom off): the route is the page.
        expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Source" });
      } finally { dispose(); }
    });
    it("Shift opens the parent block in the sidebar, Alt in the other pane, Enter from the keyboard", async () => {
      const { dispose, crumb } = await mountTrail();
      try {
        crumb("Third").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, shiftKey: true }));
        expect(rightSidebar()[0]).toMatchObject({ kind: "block", page: "Source" });
        expect(paneRouter("main").route()).toEqual({ kind: "journals" });
        crumb("First").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, altKey: true }));
        expect(layoutPaneIds()).toHaveLength(2);
        expect(paneRouter(layoutPaneIds()[1]).route()).toMatchObject({ kind: "page", name: "Source", block: expect.any(String) });
        resetPaneLayoutToSingle(snapshot());
        crumb("Second").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
        expect(paneRouter("main").route()).toMatchObject({ kind: "page", name: "Source" });
      } finally { dispose(); }
    });
  });
  it("names the embedded page with a link to it", async () => {
    vi.spyOn(backend(), "getPage").mockResolvedValue(pageRead("Source", [leaf("s1", "source row")]));
    const { root, dispose } = await mountEmbed("embed [[Source]]");
    try {
      const link = root.querySelector<HTMLAnchorElement>(".embed-page-header a.page-ref");
      expect(link?.textContent).toContain("Source");
      expect(root.textContent).toContain("source row");
    } finally {
      dispose();
    }
  });

  it("a block embed is not a page: no header", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([
      { page: "Source", kind: "page", blocks: [leaf("b1", "block row")] },
    ] as never);
    const dispose = render(() => <EmbedMacro body="embed ((b1))" />, root);
    await new Promise((resolve) => setTimeout(resolve, 50));
    try {
      expect(root.querySelector(".embed-page-header")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("a missing page keeps the raw macro text and shows no header", async () => {
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    const { root, dispose } = await mountEmbed("embed [[Nowhere]]");
    try {
      expect(root.querySelector(".embed-page-header")).toBeNull();
      expect(root.textContent).toContain("{{embed [[Nowhere]]}}");
    } finally {
      dispose();
    }
  });

  // GH #418 (comment 2): `{{embed [[PAGE🚙]]}}`. The renderer resolves any page
  // name through the one backend door unchanged, emoji included.
  it("passes an emoji page name to the backend unchanged and renders it", async () => {
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(pageRead("PAGE🚙", [leaf("e1", "emoji row")]));
    const { root, dispose } = await mountEmbed("embed [[PAGE🚙]]");
    try {
      expect(getPage).toHaveBeenCalledWith("PAGE🚙", "page");
      expect(root.querySelector(".embed-page-header")?.textContent).toContain("PAGE🚙");
      expect(root.textContent).toContain("emoji row");
    } finally {
      dispose();
    }
  });
});

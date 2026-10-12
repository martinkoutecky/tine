import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import type { BlockDto } from "../types";
import { EmbedMacro } from "./Macro";

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

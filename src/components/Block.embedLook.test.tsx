// GH #1002, #992: a whole-block embed keeps 0.9's look. Its root bullet and the guide
// under it take the embed accent, and the group adds no page spacing below its rows.
// jsdom has no layout or custom-property cascade, so this checks that the stylesheet's
// rules select the rendered embed; scripts/shot-block-embed.mjs checks the pixels.
import { readFileSync } from "node:fs";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { loadFeed, pageByName, resetStore } from "../document";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";

class AllNearObserver implements IntersectionObserver {
  readonly root = null;
  readonly rootMargin = "0px";
  readonly thresholds = [0];
  constructor(private readonly callback: IntersectionObserverCallback) {}
  disconnect(): void {}
  takeRecords(): IntersectionObserverEntry[] { return []; }
  unobserve(): void {}
  observe(target: Element): void {
    this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this);
  }
}

const appCss = readFileSync("src/styles/app.css", "utf8");
/** The selectors of every app.css rule whose declarations mention `needle`. */
function selectorsDeclaring(needle: RegExp): string[] {
  const out: string[] = [];
  for (const [, selectors, body] of appCss.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (needle.test(body)) out.push(...selectors.split(",").map((s) => s.trim()));
  }
  return out;
}

beforeAll(initParser);
beforeEach(() => vi.stubGlobal("IntersectionObserver", AllNearObserver));
afterEach(() => {
  resetStore();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

const leaf = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });
const page = (name: string, blocks: BlockDto[]): PageDto => ({ name, title: name, kind: "page", pre_block: null, blocks });

async function mountEmbed() {
  const grandkid = leaf("grandkid", "Embedded grandchild");
  const kid = leaf("kid", "Embedded child", [grandkid]);
  const target = leaf("target", "Embedded root\nid:: target", [kid]);
  const source = page("Source", [target]);
  loadFeed([page("A", [leaf("host", "{{embed ((target))}}"), leaf("after", "After")]), source]);
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) =>
    ids.map((id) => (id === "target" ? { page: "Source", kind: "page", blocks: [{ ...target, children: [] }] } : null)));
  vi.spyOn(backend(), "getPage").mockResolvedValue(source as never);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <For each={pageByName("A")?.roots}>{(id) => <Block id={id} />}</For>, root);
  const row = (id: string) => root.querySelector<HTMLElement>(`.embed-block [data-block-id="${id}"]`);
  await vi.waitFor(() => expect(row("kid")).not.toBeNull());
  return { root, dispose, row };
}

describe("a block embed's look (GH #1002, #992)", () => {
  it("the embed accent rules select the embedded root's bullet and guide, and nothing below them", async () => {
    const selectors = selectorsDeclaring(/--tine-embed-accent/).filter((s) => s.includes(".block-embed-host"));
    expect(selectors).toHaveLength(2);
    const { root, dispose, row } = await mountEmbed();
    try {
      const target = row("target")!;
      const bullet = target.querySelector(":scope > .block-main .bullet");
      const guide = target.querySelector(":scope > .block-children-container > .block-children");
      expect(bullet).not.toBeNull();
      expect(guide).not.toBeNull();
      const painted = selectors.flatMap((s) => [...root.querySelectorAll(s)]);
      expect(painted).toEqual(expect.arrayContaining([bullet, guide]));
      // The child's bullet and guide keep the ordinary colors.
      expect(painted).toHaveLength(2);
    } finally { dispose(); }
  });

  it("the embed's group takes no page-bottom spacing: the more specific padding rule on its section clears it", async () => {
    const { dispose, row } = await mountEmbed();
    try {
      const section = row("target")!.closest(".embed-block")!.querySelector(".page-section");
      // CommentSurface lends the group the page's section for its margin column.
      expect(section).not.toBeNull();
      const padding = selectorsDeclaring(/padding-bottom/).filter((s) => { try { return section!.matches(s); } catch { return false; } });
      expect(padding).toEqual([".page-section", ".comment-surface > .page-section"]);
      expect(appCss).toMatch(/\.comment-surface > \.page-section \{ padding-bottom: 0; \}/);
    } finally { dispose(); }
  });
});

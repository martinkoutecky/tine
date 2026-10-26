// GH #210 review r1 F1 (UI-GH210-HEAD-ENTER): Enter at the start of a page's
// properties must not stop the page saving. OG's page-properties editor takes
// Enter on a property line as a newline in the same field (`dwim-in-properties`)
// and its save drops the leading blank line (`wrap-parse-block`, `string/triml`).
// Driven through the real save path (pageToDto -> host -> admission). The
// in-memory host has no storage firewall; the shapes the store refuses are
// asserted on the DTO (a page-header property line outside pre_block).
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { initParser } from "../../render/parse";
import { clearSeededFacets } from "../../render/facets";
import { beginPageHeaderEdit, deleteBlock, finishPageHeaderEdit, flushPage, isDirty, markDirty, moveItem, readPageProperties, resetStore, setRaw, splitBlock } from "..";
import { doc, setDoc } from "../model";
import { loadSingle } from "../workingSet";
import { pageToDto } from "../convert";
import { bindTestHost } from "../host/wiring.test.support";
import { installDiskHost } from "../../diskHost.test.support";
import { setToasts, toasts } from "../../toasts";
import type { PageDto, PageRead } from "../../types";

beforeAll(async () => { await initParser(); });
afterEach(() => { vi.restoreAllMocks(); resetStore(); setToasts([]); });

const HEADER = "alias:: book\ntags:: reading";

async function openPage(kind: "page" | "journal", blocks: PageDto["blocks"], preBlock: string | null = HEADER) {
  const name = kind === "journal" ? "2026_10_19" : "Reading list";
  loadSingle({ name, kind, title: kind === "journal" ? "Oct 19th, 2026" : name, pre_block: preBlock, format: "md", blocks });
  clearSeededFacets();
  const files = new Map<string, PageRead>();
  const writes: PageDto[] = [];
  const host = await bindTestHost();
  installDiskHost(host, {
    read: (path) => files.get(path) ?? null,
    write: (path, dto) => {
      writes.push(structuredClone(dto));
      files.set(path, { ...structuredClone(dto), id: path, rev: `r${writes.length}` });
      return `r${writes.length}`;
    },
  });
  return { name, writes };
}
const block = (id: string, raw: string) => ({ id, raw, collapsed: false, children: [] });
const page = (name: string) => doc.pages.find((p) => p.name === name)!;
/** No page-header property line left the pre-block: the shape the store's
 * GH #163 firewall would otherwise refuse ("refusing to move page-header
 * property into outline content"). */
function headerKept(dto: PageDto | null | undefined) {
  expect(dto).toBeTruthy();
  expect(dto!.pre_block).toBe(HEADER);
  for (const b of dto!.blocks) expect(b.raw).not.toMatch(/^(alias|tags)::/m);
}

for (const kind of ["page", "journal"] as const) {
  describe(`GH #210 F1 (${kind}): Enter at the start of the page properties`, () => {
    it("the Enter key's newline at offset 0 leaves the page saving, and its later edits save", async () => {
      const { name, writes } = await openPage(kind, [block("o1", "other")]);
      const h = beginPageHeaderEdit(name)!;
      // Block.tsx: Enter on a property line is a newline in the field (OG dwim).
      setRaw(h, `\n${HEADER}`);
      finishPageHeaderEdit(h);
      expect(toasts()).toEqual([]);
      expect(page(name).roots[0]).toBe(h);
      expect(doc.byId[h].raw).toBe(HEADER); // the blank line is dropped on exit, as OG's save does
      expect(readPageProperties(name)).toEqual([["alias", "book"], ["tags", "reading"]]);
      headerKept(pageToDto(name));
      setRaw("o1", "other, edited");
      expect(await flushPage(name)).toBe(true);
      expect(isDirty(name)).toBe(false);
      headerKept(writes.at(-1));
      expect(writes.at(-1)!.blocks.map((b) => b.raw)).toEqual(["other, edited"]);
    });

    it("a blank line still in the live header (editor open) saves as the header", async () => {
      const { name, writes } = await openPage(kind, [block("o1", "other")]);
      const h = beginPageHeaderEdit(name)!;
      setRaw(h, `\n\n${HEADER}`);
      expect(readPageProperties(name)).toEqual([["alias", "book"], ["tags", "reading"]]);
      markDirty(name, "save-block");
      expect(await flushPage(name)).toBe(true);
      headerKept(writes.at(-1));
    });

    it("a split at the head of the header (any splitBlock caller) is a newline in it, not a block above it", async () => {
      const { name, writes } = await openPage(kind, [block("o1", "other")]);
      const h = beginPageHeaderEdit(name)!;
      splitBlock(h, 0);
      expect(page(name).roots).toEqual([h, "o1"]);
      expect(doc.byId[h].raw).toBe(`\n${HEADER}`);
      splitBlock(h, "alias:: book\n".length + 1); // the start of the second property line
      expect(page(name).roots).toEqual([h, "o1"]);
      expect(doc.byId[h].raw).toBe(`\nalias:: book\n\ntags:: reading`);
      finishPageHeaderEdit(h);
      expect(await flushPage(name)).toBe(true);
      expect(writes.at(-1)!.pre_block).toBe("alias:: book\n\ntags:: reading");
      expect(writes.at(-1)!.blocks.map((b) => b.raw)).toEqual(["other"]);
    });

    it("Enter at the end of the header still leaves it, into a new block below", async () => {
      const { name } = await openPage(kind, [block("o1", "other")]);
      const h = beginPageHeaderEdit(name)!;
      splitBlock(h, HEADER.length);
      expect(page(name).roots).toHaveLength(3);
      expect(page(name).roots[0]).toBe(h);
      expect(doc.byId[page(name).roots[1]].raw).toBe("");
      headerKept(pageToDto(name));
    });

    it("no move puts a block above the header (each would stop the page saving)", async () => {
      const { name } = await openPage(kind, [block("o1", "other")]);
      const h = beginPageHeaderEdit(name)!;
      expect(moveItem("o1", -1)).toBe(false);
      expect(page(name).roots).toEqual([h, "o1"]);
      headerKept(pageToDto(name));
    });
  });
}

describe("GH #210 F1: a page already in the stuck state can be repaired", () => {
  it("an older build's empty block above the header: deleting it saves the page again", async () => {
    // The older build's shape: the marked header pushed below an empty block.
    const { name, writes } = await openPage("page", [block("o1", "other")]);
    const h = beginPageHeaderEdit(name)!;
    setDoc("byId", "e0", { id: "e0", raw: "", collapsed: false, parent: null, page: name, children: [] });
    setDoc("pages", (p) => p.name === name, "roots", ["e0", h, "o1"]);
    expect(pageToDto(name)!.pre_block ?? null).toBeNull(); // the refused DTO: properties in the outline
    deleteBlock("e0");
    expect(page(name).roots).toEqual([h, "o1"]);
    expect(await flushPage(name)).toBe(true);
    headerKept(writes.at(-1));
  });

  it("the same page reopened from its crash-recovery copy (no header mark): deleting the empty block saves it", async () => {
    // After a restart the draft is ordinary blocks: an empty bullet, then the properties as a bullet.
    const { name, writes } = await openPage("page", [block("e0", ""), block("p1", HEADER), block("o1", "other")], null);
    deleteBlock("e0");
    expect(await flushPage(name)).toBe(true);
    headerKept(writes.at(-1));
  });
});

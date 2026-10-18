// GH #210 (Dieterbe, 0.8.0-beta.3): "Page-header properties must contain only
// valid key:: value lines" toasted (x2) on a journal with no page properties.
// Two ways a block that is not the page header reached that toast, one test
// each, driven through the real save path (pageToDto -> host -> admission).
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { initParser } from "../../render/parse";
import { clearSeededFacets } from "../../render/facets";
import { finishPageHeaderEdit, flushPage, insertEmptyChildBlock, markDirty, resetStore, setRaw, splitBlock } from "..";
import { doc } from "../model";
import { loadSingle } from "../workingSet";
import { pageToDto } from "../convert";
import { bindTestHost } from "../host/wiring.test.support";
import { installDiskHost } from "../../diskHost.test.support";
import { setToasts, toasts } from "../../toasts";
import type { PageDto, PageRead } from "../../types";

beforeAll(async () => { await initParser(); });
afterEach(() => { vi.restoreAllMocks(); resetStore(); setToasts([]); });

const NAME = "2026_10_18";

async function openJournal(blocks: PageDto["blocks"]) {
  loadSingle({ name: NAME, kind: "journal", title: "Oct 18th, 2026", pre_block: null, format: "md", blocks });
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
  markDirty(NAME, "save-block");
  return { writes };
}
const block = (id: string, raw: string) => ({ id, raw, collapsed: false, children: [] });
const headerToasts = () => toasts().filter((t) => /page propert|Page-header/i.test(t.message));

describe("GH #210: a block that is not the page header never raises the header toast", () => {
  it("a sheet-config first bullet is a block through a save, so its title and rows save and never toast", async () => {
    const { writes } = await openJournal([block("b1", "tine.view:: table\ntine.table-widths:: prop%3Atest=171")]);
    expect(await flushPage(NAME)).toBe(true);
    // Not folded into the preamble: no page properties exist on this page.
    expect(writes[0].pre_block ?? null).toBeNull();
    expect(writes[0].blocks.map((b) => b.raw)).toEqual(["tine.view:: table\ntine.table-widths:: prop%3Atest=171"]);
    expect(doc.byId.b1.originatedFromPageHeader).toBeUndefined();

    // Then the title and a row, as the reporter did.
    setRaw("b1", "test table\ntine.view:: table\ntine.table-widths:: prop%3Atest=171");
    expect(insertEmptyChildBlock("b1", 0)).not.toBeNull();
    finishPageHeaderEdit("b1");
    expect(headerToasts()).toEqual([]);
    expect(pageToDto(NAME)).not.toBeNull();
    expect(await flushPage(NAME)).toBe(true);
    expect(writes.at(-1)!.blocks[0].raw).toBe("test table\ntine.view:: table\ntine.table-widths:: prop%3Atest=171");
    expect(writes.at(-1)!.blocks[0].children).toHaveLength(1);
  });

  it("a block the header was pushed below is no longer the header: no toast, and its text saves", async () => {
    const { writes } = await openJournal([block("h1", "alias:: book")]);
    expect(await flushPage(NAME)).toBe(true);
    expect(writes[0].pre_block).toBe("alias:: book"); // the fold adopted h1 as the live header
    expect(doc.byId.h1.originatedFromPageHeader).toBe(true);

    splitBlock("h1", 0); // Enter at the head puts an empty block above it
    const first = doc.pages[0].roots[0];
    expect(first).not.toBe("h1");
    setRaw("h1", "alias:: book\nmore text");
    finishPageHeaderEdit("h1");
    expect(headerToasts()).toEqual([]);
  });
});

describe("GH #210: when the header toast does fire it says what is wrong", () => {
  async function markedHeader(raw: string, children: number) {
    await openJournal([block("h1", "alias:: book"), block("o1", "other")]);
    expect(await flushPage(NAME)).toBe(true);
    expect(doc.byId.h1.originatedFromPageHeader).toBe(true);
    setRaw("h1", raw);
    for (let i = 0; i < children; i++) insertEmptyChildBlock("h1", i);
    setToasts([]);
  }

  it("names the first line that is not a key:: value property, and where it is", async () => {
    await markedHeader("alias:: book\ntags:: a\ntest table", 0);
    finishPageHeaderEdit("h1");
    expect(toasts().map((t) => t.message)).toEqual([expect.stringContaining("line 3")]);
    expect(toasts()[0].message).toContain("test table");
  });

  it("truncates a long offending line", async () => {
    await markedHeader(`alias:: book\n${"x".repeat(300)}`, 0);
    finishPageHeaderEdit("h1");
    const message = toasts()[0].message;
    expect(message).toContain("line 2");
    expect(message).not.toContain("x".repeat(100));
  });

  it("says the properties block has child blocks when that is the problem", async () => {
    await markedHeader("alias:: book", 1);
    finishPageHeaderEdit("h1");
    expect(toasts().map((t) => t.message)).toEqual([expect.stringContaining("child blocks")]);
  });

  it("leaving the same invalid header twice leaves one toast, not a x2 badge", async () => {
    await markedHeader("alias:: book\nprose", 0);
    finishPageHeaderEdit("h1");
    finishPageHeaderEdit("h1");
    expect(toasts()).toHaveLength(1);
    expect(toasts()[0].count ?? 1).toBe(1);
  });
});

// CM step 2, batch 8. The window asks the Rust answers through wasm (I-12):
// - All pages leaves out what OG's `get-all-pages` drops (built-in pages, UUID
//   names, drawings), one answer for file rows and reference-only rows;
// - the prose a Markdown page shows as its first block row is the same text
//   the store reads as that page's first block for Linked References, counts
//   and rename (`page_preamble::page_preamble_content`).
import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { hidden_from_all_pages } from "./render/wasm/lsdoc_wasm";
import { splitPagePreamble } from "./editor/properties";

beforeAll(async () => {
  await initParser();
});

describe("All pages hides what OG's get-all-pages hides (REG-CM-ALL-PAGES-UUID-AND-DRAWINGS)", () => {
  it("hides UUID names, drawings and built-in pages; keeps ordinary names", () => {
    for (const name of ["6571a2b3-0000-4000-8000-00000000c8b8", "draws/2026-11-03.excalidraw", "Draws/sketch", "TODO"]) {
      expect(hidden_from_all_pages(name), name).toBe(true);
    }
    for (const name of ["Drawstring", "draws", "Contents", "6571a2b3-0000-6000-8000-00000000c8b8"]) {
      expect(hidden_from_all_pages(name), name).toBe(false);
    }
  });
});

describe("the preamble row's text is the store's first block (REG-CM-PREAMBLE-PROSE-REFERENCES)", () => {
  it("is the prose after the page header, without edge newlines", () => {
    expect(splitPagePreamble("[[Target]] in prose").content).toBe("[[Target]] in prose");
    expect(splitPagePreamble("tags:: x\n\n\nsee [[Target]]\n\n").content).toBe("see [[Target]]");
    expect(splitPagePreamble("tags:: x").content).toBeNull();
    // UTF-8 offsets from Rust map to the window's UTF-16 text.
    expect(splitPagePreamble("klíč:: é\n\nžluťoučký [[kůň]] 🐴").content).toBe("žluťoučký [[kůň]] 🐴");
  });
});

import * as wasm from "./render/wasm/lsdoc_wasm.js";
import { describe, expect, it, vi } from "vitest";
import { areaHighlightPosition, assetKey, hlsPageName, rectInPageSpace, rectWithSourceSpace } from "./pdf";

// These MUST match the Rust asset_key tests (crates/tine-core/src/pdf.rs) so the
// frontend's hls__ page name equals the page the backend reads/writes.
describe("assetKey (OG sanitize-filename parity with Rust)", () => {
  it("preserves case, `-`, `_`, spaces; keeps `-` and `_` distinct", () => {
    expect(assetKey("paper-1.pdf")).toBe("paper-1");
    expect(assetKey("paper_1.pdf")).toBe("paper_1");
    expect(assetKey("paper-1.pdf")).not.toBe(assetKey("paper_1.pdf"));
    expect(assetKey("My Paper.pdf")).toBe("My Paper");
  });
  it("strips OS-illegal chars and trailing dots/spaces; handles reserved names", () => {
    expect(assetKey("a/b:c.pdf")).toBe("abc");
    expect(assetKey("Re: notes?.pdf")).toBe("Re notes");
    expect(assetKey("Book.PDF")).toBe("Book");
    expect(assetKey("draft. .pdf")).toBe("draft");
    expect(assetKey("CON.pdf")).toBe("");
  });
  it("preserves the preview suffix policy for mixed-case and Unicode filenames", () => {
    expect(assetKey("Paper.PdF")).toBe("Paper");
    expect(assetKey("résumé.pdf")).toBe("résumé");
    expect(assetKey("COM7.extra.pdf")).toBe("");
  });
  it("hlsPageName composes the key", () => {
    expect(hlsPageName("My Paper.pdf")).toBe("hls__My Paper");
  });
});

describe("PDF highlight coordinate interoperability", () => {
  it("rescales current Logseq coordinates from their creation viewport", () => {
    const converted = rectInPageSpace({
      left: 292.1,
      top: 488.4,
      width: 263.4,
      height: 46.7,
      source_width: 822,
      source_height: 1063.7,
    }, { w: 612, h: 792 });
    expect(converted.left).toBeCloseTo(217.48, 2);
    expect(converted.top).toBeCloseTo(363.65, 2);
    expect(converted.width).toBeCloseTo(196.11, 2);
    expect(converted.height).toBeCloseTo(34.77, 2);
  });

  it("leaves old Tine page-space coordinates in place and annotates them for writing", () => {
    const old = { left: 72, top: 90, width: 120, height: 18 };
    expect(rectInPageSpace(old, { w: 612, h: 792 })).toEqual(old);
    expect(rectWithSourceSpace(old, { w: 612, h: 792 })).toEqual({
      ...old,
      source_width: 612,
      source_height: 792,
    });
  });

  it("uses OG's empty rect list for an area highlight", () => {
    const bounding = {
      left: 10,
      top: 20,
      width: 30,
      height: 40,
      source_width: 612,
      source_height: 792,
    };
    expect(areaHighlightPosition(3, bounding)).toEqual({
      page: 3,
      bounding,
      rects: [],
    });
  });
});

it("highlights sharing a filename reuse the native identity, including empty keys", () => {
  const read = vi.spyOn(wasm, "pdf_asset_key");
  try {
    for (let i = 0; i < 100; i++) {
      expect(assetKey("cost-probe-Paper.pdf")).toBe("cost-probe-Paper");
      expect(assetKey("NUL.cache-probe.pdf")).toBe("");
    }
    expect(read).toHaveBeenCalledTimes(2);
    for (let i = 0; i < 128; i++) assetKey(`bounded-Paper-${i}.pdf`);
    expect(assetKey("cost-probe-Paper.pdf")).toBe("cost-probe-Paper");
    expect(read).toHaveBeenCalledTimes(131);
  } finally { read.mockRestore(); }
});

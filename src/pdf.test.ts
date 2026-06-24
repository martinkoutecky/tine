import { describe, expect, it } from "vitest";
import { assetKey, hlsPageName } from "./pdf";

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
  it("hlsPageName composes the key", () => {
    expect(hlsPageName("My Paper.pdf")).toBe("hls__My Paper");
  });
});

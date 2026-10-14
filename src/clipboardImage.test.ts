import { describe, expect, it } from "vitest";
import { ASSET_INGRESS_MAX_BYTES, clipboardSizeAllowed } from "./clipboardImage";

describe("clipboard size admission (GH #654)", () => {
  it.each([undefined, null, 0, -1, 12, ASSET_INGRESS_MAX_BYTES])("allows unknown or bounded size %s for a bounded read", (size) => {
    expect(clipboardSizeAllowed(size)).toBe(true);
  });
  it("refuses a known oversized image before reading", () => {
    expect(clipboardSizeAllowed(ASSET_INGRESS_MAX_BYTES + 1)).toBe(false);
  });
});

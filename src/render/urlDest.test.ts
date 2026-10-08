import { expect, it } from "vitest";
import { urlDest } from "./urlDest";
it("preserves browser destinations, including the empty-protocol policy", () => {
  for (const type of ["page_ref", "block_ref", "search", "file", "embed_data"] as const) expect(urlDest({ type, v: "x" })).toBe("x");
  expect(urlDest({ type: "complex", protocol: "https", link: "example.org" })).toBe("https://example.org");
  expect(urlDest({ type: "complex", protocol: "", link: "x" })).toBe("x");
  expect(urlDest({ type: "complex", link: "x" })).toBe("x");
  expect(urlDest({ type: "complex" })).toBe("");
});

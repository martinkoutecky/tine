import { expect, it } from "vitest";
import { urlDest } from "./urlDest";
it("preserves browser destinations, including the empty-protocol policy", () => {
  for (const type of ["page_ref", "block_ref", "search", "file", "embed_data"] as const) expect(urlDest({ type, v: "x" })).toBe("x");
  expect(urlDest({ type: "complex", protocol: "https", link: "example.org" })).toBe("https://example.org");
  expect(urlDest({ type: "complex", protocol: "", link: "x" })).toBe("x");
  expect(urlDest({ type: "complex", link: "x" })).toBe("x");
  expect(urlDest({ type: "complex" })).toBe("");
});
it("does not invent a separator in a one-letter Org protocol", () => {
  expect(urlDest({ type: "complex", protocol: "C", link: "%5Cdocs%5Cnote.pdf" }, "org")).toBe("C:%5Cdocs%5Cnote.pdf");
  expect(urlDest({ type: "complex", protocol: "C", link: "docs/note.pdf" }, "org")).toBe("C:docs/note.pdf");
  const drive = { type: "complex", protocol: "C", link: "docs/note.pdf" } as const;
  expect(urlDest(drive, "org", "[[C://docs/note.pdf][Open]]")).toBe("C://docs/note.pdf");
  expect(urlDest(drive, "org", "[[C:docs/note.pdf][C://docs/note.pdf]]")).toBe("C:docs/note.pdf");
});

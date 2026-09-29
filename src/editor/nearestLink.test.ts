import { describe, expect, it } from "vitest";
import { nearestLink } from "./nearestLink";

// GH #274. Transcribed from OG `thingatpt.cljs extract-nearest-link-from-text`;
// these cases encode OG's actual rule, including the parts that are surprising.
describe("nearestLink", () => {
  const at = (text: string, caret: number, includeUrls = false) =>
    nearestLink(text, caret, { includeUrls });

  it("finds a page ref the caret is inside", () => {
    const text = "see [[Some Page]] for more";
    expect(at(text, text.indexOf("Some") + 2)).toMatchObject({ kind: "page", value: "Some Page" });
  });

  it("finds the link even when the caret is nowhere near it", () => {
    // OG picks the NEAREST candidate in the whole block, not one under the
    // caret. A caret at the end of a block with one link still follows it.
    const text = "see [[Some Page]] for more";
    expect(at(text, text.length)).toMatchObject({ kind: "page", value: "Some Page" });
  });

  it("prefers the link the caret is inside over a closer-looking neighbour", () => {
    const text = "[[First]] and [[Second]]";
    expect(at(text, text.indexOf("Second") + 1)).toMatchObject({ value: "Second" });
    expect(at(text, 3)).toMatchObject({ value: "First" });
  });

  it("picks the nearer of two links when inside neither", () => {
    const text = "[[First]]           x [[Second]]";
    expect(at(text, text.indexOf("x"))).toMatchObject({ value: "Second" });
  });

  it("strips a tag's leading hash", () => {
    expect(at("a #project note", 4)).toMatchObject({ kind: "tag", value: "project" });
  });

  it("returns a block ref's uuid", () => {
    const text = "see ((6a55b643-0000-4000-8000-000000000000))";
    expect(at(text, 10)).toMatchObject({
      kind: "block",
      value: "6a55b643-0000-4000-8000-000000000000",
    });
  });

  it("finds a URL only when URLs are requested", () => {
    const text = "docs at https://example.test/a/b";
    expect(at(text, 12)).toBeNull();
    expect(at(text, 12, true)).toMatchObject({ kind: "url", value: "https://example.test/a/b" });
  });

  it("prefers a page ref over a URL the caret is further from", () => {
    const text = "[[Notes]] and https://example.test/x";
    expect(at(text, 3, true)).toMatchObject({ kind: "page", value: "Notes" });
  });

  it("returns null when the block has no link at all", () => {
    expect(at("just some prose", 5, true)).toBeNull();
  });

  it("returns null for an empty ref rather than navigating nowhere", () => {
    expect(at("[[   ]]", 3)).toBeNull();
  });

  it("reports the match offsets", () => {
    const text = "see [[Some Page]] here";
    expect(at(text, 8)).toMatchObject({ start: 4, end: 17 });
  });
});

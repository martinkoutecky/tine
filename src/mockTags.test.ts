// GH #256 second-order guard. The lookbehind that broke startup on old WebKit
// had to be replaced by something with the SAME match behaviour, and the
// obvious replacement does not have it.
//
// `(?<!\[)#tag` tests the preceding character WITHOUT consuming it. Rewriting
// that as a leading group — `(^|[^\[])#tag` — reads as equivalent but consumes
// the character, so with a global regex a tag written immediately after another
// one has nothing left to match its prefix against and is silently dropped.
// These cases are exactly the ones that divergence shows up in.
import { describe, expect, it } from "vitest";
import { tagsOf } from "./mock";
describe("mock tag extraction (GH #256 regression)", () => {
  const extract = tagsOf;

  it("still ignores the [#A] priority token, which is what the lookbehind was for", () => {
    expect(extract("[#A] TODO #real")).toEqual(["real"]);
    expect(extract("[#B]")).toEqual([]);
    expect(extract("#a[#B]#c")).toEqual(["a", "c"]);
  });

  it("does not drop a tag written immediately after another", () => {
    expect(extract("#a#b")).toEqual(["a", "b"]);
    expect(extract("a#one#two")).toEqual(["one", "two"]);
    expect(extract("#[[x]]#y")).toEqual(["x", "y"]);
    expect(extract("x [#A] #t1#t2 end")).toEqual(["t1", "t2"]);
  });

  it("keeps the ordinary cases intact", () => {
    expect(extract("#one #two")).toEqual(["one", "two"]);
    expect(extract("text #[[Long Tag]] and #short")).toEqual(["Long Tag", "short"]);
    expect(extract("no tags here")).toEqual([]);
    expect(extract("##double")).toEqual(["double"]);
  });
});

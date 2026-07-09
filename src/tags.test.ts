import { describe, expect, it } from "vitest";
import { tagRef } from "./tags";

describe("tagRef", () => {
  it("keeps parser-safe names bare, including non-ASCII", () => {
    expect(tagRef("sheets")).toBe("#sheets");
    expect(tagRef("a/b_c.d-e")).toBe("#a/b_c.d-e");
    expect(tagRef("čeština")).toBe("#čeština"); // unicode letters are valid bare tags
    expect(tagRef("výuka2026")).toBe("#výuka2026");
  });

  it("brackets names the tag lexer would truncate or misparse", () => {
    expect(tagRef("multi word")).toBe("#[[multi word]]");
    expect(tagRef("a,b")).toBe("#[[a,b]]"); // hard stop
    expect(tagRef("c#")).toBe("#[[c#]]"); // hard stop
    expect(tagRef("why?")).toBe("#[[why?]]");
    expect(tagRef("re:search")).toBe("#[[re:search]]");
    expect(tagRef("quo'te")).toBe("#[[quo'te]]");
    expect(tagRef("br[ack]et")).toBe("#[[br[ack]et]]");
    expect(tagRef("trail.")).toBe("#[[trail.]]"); // trailing delimiter run is stripped bare
    expect(tagRef("trail;")).toBe("#[[trail;]]");
    expect(tagRef("")).toBe("#[[]]");
  });
});

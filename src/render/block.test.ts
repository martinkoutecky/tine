import { describe, expect, it } from "vitest";
import { aliasNames, isPropertyLine, blockView } from "./block";

describe("aliasNames", () => {
  it("parses a comma-separated alias:: line", () => {
    expect(aliasNames("alias:: Foo, Bar Baz, qux")).toEqual(["Foo", "Bar Baz", "qux"]);
  });
  it("is case-insensitive on the key and ignores other properties", () => {
    expect(aliasNames("tags:: x\nAlias:: Foo\npublic:: true")).toEqual(["Foo"]);
  });
  it("returns [] for no alias / empty / null", () => {
    expect(aliasNames("tags:: x")).toEqual([]);
    expect(aliasNames("")).toEqual([]);
    expect(aliasNames(null)).toEqual([]);
    expect(aliasNames("alias:: , ,")).toEqual([]);
  });
});

describe("isPropertyLine", () => {
  it("accepts a key:: value line and rejects prose", () => {
    expect(isPropertyLine("alias:: foo")).toBe(true);
    expect(isPropertyLine("just some text")).toBe(false);
    expect(isPropertyLine(":: leading")).toBe(false);
  });
});

describe("blockView SCHEDULED/DEADLINE", () => {
  it("treats a timestamp-only line as a marker and strips it from the body", () => {
    const v = blockView("TODO ship it\nSCHEDULED: <2026-07-06 Mon>");
    expect(v.scheduled).toBe("2026-07-06 Mon");
    expect(v.lines.join("\n")).toBe("ship it");
  });
  it("keeps the badge AND renders trailing text after the timestamp (lenient)", () => {
    const v = blockView("TODO \nSCHEDULED: <2026-07-06 Mon> #email ADS1 students");
    expect(v.scheduled).toBe("2026-07-06 Mon"); // badge kept
    expect(v.lines.join("\n")).toContain("#email ADS1 students"); // text shown as body
    expect(v.lines.join("\n")).not.toContain("SCHEDULED:"); // the prefix is hidden
  });
});

import { describe, expect, it } from "vitest";
import { aliasNames, isPropertyLine } from "./block";

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

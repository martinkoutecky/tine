import { describe, expect, it } from "vitest";
import { aliasNames, isPropertyLine, pageProperties, blockView } from "./block";

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
  it("reads org #+ALIAS: / :alias: drawer", () => {
    expect(aliasNames("#+TITLE: P\n#+ALIAS: foo, bar", "org")).toEqual(["foo", "bar"]);
    expect(aliasNames(":PROPERTIES:\n:alias: baz\n:END:", "org")).toEqual(["baz"]);
  });
});

describe("pageProperties", () => {
  it("markdown key:: value lines", () => {
    expect(pageProperties("title:: P\ntags:: a, b")).toEqual([
      ["title", "P"],
      ["tags", "a, b"],
    ]);
  });
  it("org #+KEY: directives and :PROPERTIES: drawer (keys lowercased)", () => {
    expect(pageProperties("#+TITLE: org-sink\n#+FILETAGS: :demo:org:", "org")).toEqual([
      ["title", "org-sink"],
      ["filetags", ":demo:org:"],
    ]);
    expect(pageProperties(":PROPERTIES:\n:key: value\n:END:", "org")).toEqual([["key", "value"]]);
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
    expect(v.marker).toBe("TODO");
    // No spurious blank line: the marker-only first line is dropped, body is one line.
    expect(v.lines).toEqual(["#email ADS1 students"]);
    expect(v.lines.join("\n")).not.toContain("SCHEDULED:"); // the prefix is hidden
  });
  it("finds SCHEDULED inline on the same line as the marker (no own line needed)", () => {
    const v = blockView("TODO SCHEDULED: <2026-07-06 Mon> do the thing");
    expect(v.scheduled).toBe("2026-07-06 Mon");
    expect(v.marker).toBe("TODO");
    expect(v.lines).toEqual(["do the thing"]); // token stripped, text flows after marker
  });
  it("finds DEADLINE inline too", () => {
    const v = blockView("DEADLINE: <2026-07-06 Mon> pay rent");
    expect(v.deadline).toBe("2026-07-06 Mon");
    expect(v.lines).toEqual(["pay rent"]);
  });

});

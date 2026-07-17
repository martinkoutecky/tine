import { describe, expect, it } from "vitest";
import { aliasNames, isPropertyLine, pageProperties, visibleBody } from "./block";

describe("visibleBody (body text for labels / reference render)", () => {
  it("drops real property lines but keeps a fenced key:: as code content", () => {
    const body = visibleBody("title:: Real\n```\nlang:: rust\nlet x = 1;\n```\nfoo:: bar").join("\n");
    expect(body).not.toContain("title:: Real"); // real block property → not body text
    expect(body).not.toContain("foo:: bar");
    expect(body).toContain("lang:: rust"); // fenced → stays as code content
  });
});

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
  it("accepts aliases:: and full-width separators while quoted values suppress refs", () => {
    expect(aliasNames("aliases:: Foo， Bar, Baz")).toEqual(["Foo", "Bar", "Baz"]);
    expect(aliasNames('alias:: "Foo, Bar"')).toEqual([]);
  });
});

describe("pageProperties", () => {
  it("markdown key:: value lines", () => {
    expect(pageProperties("title:: P\ntags:: a, b")).toEqual([
      ["title", "P"],
      ["tags", "a, b"],
    ]);
  });
  it("only renders the canonical header prefix, never later prose/fence lookalikes", () => {
    expect(pageProperties("Intro\ncustom:: not-a-header")).toEqual([]);
    expect(pageProperties("```\ncustom:: not-a-header\n```")).toEqual([]);
    expect(pageProperties("klíč:: hodnota\n\ncustom/key:: value\n\nIntro\nlater:: body")).toEqual([
      ["klíč", "hodnota"],
      ["custom/key", "value"],
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

describe("visibleBody strips header chrome from the body text", () => {
  it("strips marker / priority / heading prefix from the first line", () => {
    expect(visibleBody("TODO [#A] ## ship it")).toEqual(["ship it"]);
    expect(visibleBody("DOING write the doc")).toEqual(["write the doc"]);
  });
  it("removes a standalone SCHEDULED/DEADLINE planning line (it's a date badge)", () => {
    expect(visibleBody("TODO ship it\nSCHEDULED: <2026-07-06 Mon>")).toEqual(["ship it"]);
    expect(visibleBody("DEADLINE: <2026-07-06 Mon>\npay rent")).toEqual(["pay rent"]);
  });
  it("keeps an inline (non-standalone) SCHEDULED as body text (not a real timestamp)", () => {
    // Mirrors lsdoc: only a standalone planning line is a Timestamp; inline stays text.
    expect(visibleBody("do SCHEDULED: <2026-07-06 Mon> the thing")).toEqual([
      "do SCHEDULED: <2026-07-06 Mon> the thing",
    ]);
  });
});

import { describe, expect, it } from "vitest";
import { aliasNamesOf, isPropertyLine, isRenderHiddenProp, pageProperties, visibleBody } from "./block";
import type { Format } from "../types";

const aliasNames = (text: string | null, format?: Format) => aliasNamesOf(pageProperties(text, format));

describe("visibleBody (body text for labels / reference render)", () => {
  it("uses whole-block task recognition before extracting the first visible line", () => {
    expect(visibleBody("\n\nTODO buy milk")).toEqual(["buy milk"]);
    expect(visibleBody("TODO\nmore detail")).toEqual(["TODO", "more detail"]);
    expect(visibleBody("TODO TODO buy milk")).toEqual(["TODO buy milk"]);
    expect(visibleBody("id:: example\nTODO buy milk")).toEqual(["TODO buy milk"]);
  });
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
    expect(pageProperties("alias:: Ghost\n#+ALIAS: Novel", "org")).toEqual([["alias", "Novel"]]);
    expect(pageProperties(":PROPERTIES:\n:alias: Vacant\n:END:", "org")).toEqual([["alias", "Vacant"]]);
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

describe("visibleBody removes exactly the metadata lsdoc accepted (I-12)", () => {
  it("drops one planning line carrying SCHEDULED and DEADLINE together", () => {
    expect(visibleBody("first\nSCHEDULED: <2026-01-01 Thu> DEADLINE: <2026-01-02 Fri>\nbody")).toEqual([
      "first",
      "body",
    ]);
  });
  it("keeps the text of a drawer that never closes (it is body, not metadata)", () => {
    expect(visibleBody("first\n:LOGBOOK:\nbody after the open drawer")).toEqual([
      "first",
      ":LOGBOOK:",
      "body after the open drawer",
    ]);
  });
  it("keeps a CLOCK-looking line outside any drawer", () => {
    expect(visibleBody("first\nCLOCK: not really a clock\nbody")).toEqual([
      "first",
      "CLOCK: not really a clock",
      "body",
    ]);
  });
  it("still drops a closed PROPERTIES drawer and a real logbook drawer", () => {
    expect(visibleBody("first\n  :PROPERTIES:\n  :a: b\n  :END:\nbody")).toEqual(["first", "body"]);
    expect(
      visibleBody("TODO first\n:LOGBOOK:\nCLOCK: [2026-01-01 Thu 10:00:00]--[2026-01-01 Thu 10:05:00] =>  00:05:00\n:END:\nbody"),
    ).toEqual(["first", "body"]);
  });
});

describe("isRenderHiddenProp is the Rust render_facets answer (I-12)", () => {
  it("hides the built-ins, tine.* and logseq.table.*, and shows other logseq.* keys", () => {
    for (const key of ["id", "Title", "hl-color", "Created_At", "tine.view", "logseq.table.version"]) {
      expect(isRenderHiddenProp(key), key).toBe(true);
    }
    for (const key of ["logseq.custom", "status", "public"]) expect(isRenderHiddenProp(key), key).toBe(false);
  });
  it("applies the graph's :block-hidden-properties after the shared key fold", () => {
    expect(isRenderHiddenProp("mine", ["Mine"])).toBe(true);
    expect(isRenderHiddenProp("other", ["Mine"])).toBe(false);
  });
});

// OG-P12B (Martin 2026-10-01): page headers, code fences and task markers all follow the
// parser (OG/mldoc). Each test names the user-visible behaviour it protects.
import { describe, expect, it, vi } from "vitest";
import { isPageHeaderPropertiesOnly, pagePropertyEntries, splitPagePreamble, caretOnPropertyLine } from "./editor/properties";
import { caretInFence, codeFences } from "./editor/fences";
import { codeBodyProjection, codeFenceOnly } from "./editor/codeFence";
import { calcSource } from "./editor/calc";
import { visibleBody } from "./render/block";
import { matchLeadingMarker, headerTokens } from "./markers";
import { cycleMarker, setMarker } from "./editor/marker";
import { setPriority } from "./editor/format";
import { blockRegions } from "./render/parse";
import { detectTrigger } from "./editor/autocomplete";

describe("D05: the Markdown page header is the parser's", () => {
  it("a no-space `key::value` line is not a page property", () => {
    expect(pagePropertyEntries("alias:: book\nplugin.key::value", "md").map((e) => e.key)).toEqual(["alias"]);
    expect(isPageHeaderPropertiesOnly("alias:: book\nplugin.key::value")).toBe(false);
    expect(isPageHeaderPropertiesOnly("klic::value")).toBe(false);
    expect(splitPagePreamble("klic::value")).toEqual({ properties: null, content: "klic::value", remainder: "klic::value" });
  });
  it("Unicode keys, empty-line gaps and a UTF-16 end offset agree with the parser", () => {
    const raw = "klíč:: hodnota\n\nžluť:: 値";
    expect(isPageHeaderPropertiesOnly(raw)).toBe(true);
    expect(splitPagePreamble(`${raw}\n\nprose`)).toEqual({ properties: raw, content: "prose", remainder: "\n\nprose" });
  });
  it("fenced property-shaped lines, indented lines and `#tag::` lines are not header properties", () => {
    expect(pagePropertyEntries("title:: A\n```\nx:: y\n```", "md").map((e) => e.key)).toEqual(["title"]);
    expect(pagePropertyEntries(" indented:: x", "md")).toEqual([]);
    expect(pagePropertyEntries("#tag:: x", "md")).toEqual([]);
    expect(caretOnPropertyLine("klic::value", 3)).toBe(false);
  });
});

describe("D07: fences follow the parser's literal ranges, not CommonMark", () => {
  it("a closer of any length or character closes the container (four-run opener, three-run closer)", () => {
    const raw = "````text\nalpha\n```\nafter";
    expect(codeFences(raw).map((f) => [f.closed, f.start, f.end])).toEqual([[true, 0, 19]]);
    expect(caretInFence(raw, raw.indexOf("alpha"))).toBe(true);
    expect(caretInFence(raw, raw.indexOf("after"))).toBe(false);
  });
  it("a tilde closer closes a backtick opener (both read through one container)", () => {
    expect(codeFenceOnly("```js\nx\n~~~", "md")).not.toBeNull();
    expect(codeBodyProjection("```js\nx\n~~~", "md")).toEqual({ open: "```js\n", body: "x", close: "\n~~~", lang: "js" });
  });
  it("`- ```js` is not a fence (lsdoc does not report one; recorded, not worked around)", () => {
    // `- ```js` opens nothing; the only fence is the final lone run, an unclosed editor-state fence.
    const raw = "- ```js\nx\n```";
    expect(blockRegions(raw).literal_blocks).toEqual([]);
    expect(codeFences(raw).map((f) => [f.closed, f.start])).toEqual([[false, raw.lastIndexOf("```")]]);
    expect(caretInFence(raw, raw.indexOf("x"))).toBe(false);
  });
  it("an unclosed fence still behaves as an open fence while typing (named editor policy)", () => {
    const raw = "intro\n```js\nconst x = 1";
    const [fence] = codeFences(raw);
    expect(fence.closed).toBe(false);
    expect(caretInFence(raw, raw.indexOf("const"))).toBe(true);
    expect(codeBodyProjection(raw, "md")).toBeNull();
  });
  it("org #+BEGIN_SRC is a container in Markdown text too, and a mismatched org closer is not one", () => {
    expect(codeFences("#+BEGIN_SRC python\nx\n#+END_SRC", "md").map((f) => f.closed)).toEqual([true]);
    expect(codeFences("#+BEGIN_SRC python\nx\n#+END_EXAMPLE", "org").map((f) => f.closed)).toEqual([false]);
    expect(codeBodyProjection("#+BEGIN_SRC python\nx\n#+END_EXAMPLE", "org")).toBeNull();
  });
  it("```calc reads the parser's source container; visibleBody keeps container lines as content", () => {
    expect(calcSource("```calc\n1 + 1\n```")).toBe("1 + 1");
    expect(calcSource("```calc\n1 + 1")).toBe("1 + 1");
    expect(visibleBody("```\nfoo:: ghost\nSCHEDULED: <2026-10-01 Thu>\n```\nfoo:: real")).toEqual([
      "```", "foo:: ghost", "SCHEDULED: <2026-10-01 Thu>", "```",
    ]);
    // A four-run opener closed by a three-run: the line after is metadata again.
    expect(visibleBody("````\nx\n```\nfoo:: real")).toEqual(["````", "x", "```"]);
  });
  it("autocomplete treats a property-shaped line inside a closed container as code", () => {
    const inside = "```\nfoo::";
    expect(detectTrigger(inside, inside.length)).toBeNull();
    const after = "```\nx\n```\nfoo::";
    expect(detectTrigger(after, after.length)?.kind).toBe("property-name");
  });
});

describe("D13: one accepted marker/priority answer for readers and writers", () => {
  it("U+0085 is parser whitespace (accepted), U+FEFF is not", () => {
    expect(matchLeadingMarker("\u0085TODO x")).toEqual({ marker: "TODO", start: 1, end: 5 });
    expect(matchLeadingMarker("﻿TODO x")).toBeNull();
    expect(cycleMarker("\u0085TODO x", "todo").raw).toBe("\u0085DOING x");
    expect(cycleMarker("﻿TODO x", "todo").raw.endsWith("TODO x")).toBe(true);
    expect(setMarker("\u0085TODO x", "DONE")).toBe("\u0085DONE x");
  });
  it("priority is read and written on the parser's span (after the marker only)", () => {
    expect(headerTokens("TODO [#A] x").priority).toEqual({ text: "A", start: 5, end: 9 });
    expect(headerTokens("héé TODO [#A]").priority).toBeNull();
    expect(setPriority("TODO [#A] x", "B")).toBe("TODO [#B] x");
    expect(setPriority("TODO [#A] x", null)).toBe("TODO x");
    expect(setPriority("TODO x [#A]", "B")).toBe("TODO [#B] x [#A]");
    expect(visibleBody("TODO [#A] x")).toEqual(["x"]);
  });
});

describe("D13: the demo mock reads the parser only once it is ready", () => {
  it("building the mock before the parser is initialised neither throws nor freezes empty facets", async () => {
    vi.resetModules();
    const parse = await import("./render/parse");
    expect(parse.parserReady()).toBe(false);
    const mock = await import("./mock");
    const backend = mock.mockBackend();
    await parse.initParser();
    const page = (await backend.getPage("Jun 14th, 2026", "journal"))!;
    const todo = JSON.parse(JSON.stringify(page)).blocks.find((b: { raw: string }) => b.raw.startsWith("TODO [#A] Ship"));
    expect(todo?.marker).toBe("TODO");
    expect(todo?.priority).toBe("A");
  });
});

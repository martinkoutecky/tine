// @vitest-environment jsdom
// GH #208 / #338: in-page find & replace. The plan is pure (what one block becomes);
// the entry points (inpageReplace.host.dom.test.tsx) write through setRaw inside
// one withUndoUnit and the page host's ordinary save. UI-INPAGE-FIND-REPLACE-208.
import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { planBlockReplacement } from "./inpageReplace";

const ID = "6501f00a-0000-4000-8000-000000000000";
// The planner's parse gate alone: every span counts as drawn as itself. The
// screen's answer is pinned in inpageReplace.shown.dom.test.tsx.
const DRAWN = () => true;
const plan = (raw: string, query: string, replacement: string, ordinals: readonly number[] | "all" = "all", format: "md" | "org" = "md") =>
  planBlockReplacement(raw, format, query, replacement, ordinals, true, DRAWN);

beforeAll(async () => { await initParser(); });

describe("planBlockReplacement: only what find shows, only where the editor edits", () => {
  it("replaces an OG-layout block without skipping or reordering its hidden rows", () => {
    for (const raw of ["foo\ncard-repeats:: 2\nowner:: Ada\nbody", "foo\ncard-repeats:: 2\nowner:: Ada\nbody foo"]) {
      expect(plan(raw, "foo", "bar")).toEqual({ raw: raw.replaceAll("foo", "bar"), replaced: raw.endsWith("foo") ? 2 : 1, skipped: 0 });
    }
  });

  it("replaces visible text and a link label but never the link target", () => {
    // A raw-text replace would rewrite https://foo.com as well.
    expect(plan("foo [label foo](https://foo.com) foo", "foo", "bar"))
      .toEqual({ raw: "bar [label bar](https://foo.com) bar", replaced: 3, skipped: 0 });
  });

  it("never rewrites a hidden property line", () => {
    const raw = `true story\ncollapsed:: true\nid:: ${ID}`;
    expect(plan(raw, "true", "false")).toEqual({ raw: `false story\ncollapsed:: true\nid:: ${ID}`, replaced: 1, skipped: 0 });
    expect(plan(raw, "6501f00a", "x").replaced).toBe(0);
  });

  it("replaces task text without moving or replacing its hidden LOGBOOK", () => {
    const raw = "DOING foo\ncollapsed:: true\n:LOGBOOK:\nCLOCK: [2026-10-21 Wed 08:00:00]\n:END:\nDEADLINE: <2026-10-22 Thu>";
    expect(plan(raw, "foo", "bar")).toEqual({raw: raw.replace("foo", "bar"), replaced: 1, skipped: 0});
    expect(plan(raw, "CLOCK", "lost").replaced).toBe(0);
  });

  it("replaces only the requested occurrence", () => {
    expect(plan("foo foo foo", "foo", "bar", [1]).raw).toBe("foo bar foo");
  });

  it("matches as find does (case and accents folded) and keeps the markup around a match", () => {
    expect(plan("**Foo** foo café", "foo", "bar").raw).toBe("**bar** bar café");
    expect(plan("café", "cafe", "tea").raw).toBe("tea");
  });

  it("proves the occurrence instead of trusting the first equal letters", () => {
    // Rendered "xfoo": an order-only alignment lands on the link target "foo".
    expect(plan("[x](foo)foo", "foo", "bar")).toEqual({ raw: "[x](foo)bar", replaced: 1, skipped: 0 });
  });

  it("an empty replacement deletes the match", () => {
    expect(plan("a-drop-b", "drop", "").raw).toBe("a--b");
  });
});

// Shown text step 2a (REG-FIND-REPLACE-WRITES-ONLY-SHOWN-TEXT): Find counts the
// literal of a macro and a block reference id, but they are not text drawn as
// itself, so Replace never writes them. Fails on develop, which rewrote them.
describe.each(["md", "org"] as const)("%s: Replace writes only exact runs of the parse", (format) => {
  const UUID = "6501f00a-0000-4000-8000-00000000000e";
  it("an embed macro's name and target are never written; the plain word after it is", () => {
    expect(plan("{{embed [[Foo]]}} Foo", "e", "Bar", "all", format))
      .toEqual({ raw: "{{embed [[Foo]]}} Foo", replaced: 0, skipped: 2 });
    expect(plan("{{embed [[Foo]]}} Foo", "Foo", "X", "all", format))
      .toEqual({ raw: "{{embed [[Foo]]}} X", replaced: 1, skipped: 1 });
    expect(plan("{{embed [[Foo]]}} Foo", "Foo", "X", [0], format))
      .toEqual({ raw: "{{embed [[Foo]]}} Foo", replaced: 0, skipped: 1 });
  });

  it("a block reference's id is never written; the plain letter after it is", () => {
    const raw = `((${UUID})) e`;
    const all = plan(raw, "e", "X", "all", format);
    expect(all.raw).toBe(`((${UUID})) X`);
    expect(all.replaced).toBe(1);
    expect(all.skipped).toBeGreaterThan(0);
  });

  it("plain prose, emphasis text and an inline literal body are written", () => {
    expect(plan("a needle b", "needle", "X", "all", format).raw).toBe("a X b");
    expect(plan("a needle b", "needle", "", "all", format).raw).toBe("a  b");
    if (format === "md") {
      expect(plan("a `needle` b", "needle", "X", "all", format).raw).toBe("a `X` b");
      expect(plan("**needle** b", "needle", "X", "all", format).raw).toBe("**X** b");
    } else {
      expect(plan("a =needle= b", "needle", "X", "all", format).raw).toBe("a =X= b");
      expect(plan("a ~needle~ b", "needle", "X", "all", format).raw).toBe("a ~X~ b");
      expect(plan("*needle* b", "needle", "X", "all", format).raw).toBe("*X* b");
    }
  });

  it("an emoji the user searched for is its own source bytes", () => {
    expect(plan("a 😄 b", "😄", "X", "all", format)).toEqual({ raw: "a X b", replaced: 1, skipped: 0 });
  });

  it("the gate adds no parse: one parse of the block and one proof render, as before", () => {
    const stats = () => (window as unknown as { __tineParseStats?: { misses: number } }).__tineParseStats?.misses ?? 0;
    const raw = `cost probe ${format} needle x needle`;
    const before = stats();
    expect(plan(raw, "needle", "X", "all", format).replaced).toBe(2);
    expect(stats() - before).toBe(2);
  });
});

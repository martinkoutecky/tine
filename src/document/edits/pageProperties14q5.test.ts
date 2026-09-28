// og 14 Q5 follow-up: one answerer for "this page's properties" (Reader B
// blocker B1 + G3 follow-up) and case-insensitive block property removal (G3
// required neighbor). The panel lists what the page renders, and the writer
// edits the line the list came from.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { blockProperty, pageByName, readPageProperties, readPageProperty, resetStore, setBlockProperty, setPageProperty } from "..";
import { doc } from "../model";
import { loadSingle } from "../workingSet";
import { pageProperties } from "../../render/block";
import { initParser } from "../../render/parse";
import { clearSeededFacets, facetsOf } from "../../render/facets";
import type { Format } from "../../types";

function load(name: string, preBlock: string | null, raws: string[] = ["body"], format: Format = "md") {
  loadSingle({
    name,
    kind: "page",
    title: name,
    pre_block: preBlock,
    format,
    blocks: raws.map((raw, i) => ({ id: `${name}-${i}`, raw, collapsed: false, children: [] })),
  });
  clearSeededFacets();
}

const pre = (name: string) => pageByName(name)?.preBlock ?? null;

beforeAll(async () => {
  await initParser();
});

afterEach(() => resetStore());

describe("readPageProperties is the rendered page-property set (B1)", () => {
  it("does not list a key:: line inside a fenced code block or after prose, and never rewrites it", () => {
    const preBlock = "status:: draft\n\nIntro\n```\nfoo:: bar\n```\nlate:: no";
    load("Fenced", preBlock);
    expect(readPageProperties("Fenced")).toEqual([["status", "draft"]]);
    expect(readPageProperty("Fenced", "foo")).toBeNull();
    setPageProperty("Fenced", "foo", "x");
    expect(pre("Fenced")).toBe("foo:: x\nstatus:: draft\n\nIntro\n```\nfoo:: bar\n```\nlate:: no");
  });

  it("is derived from the renderer's pageProperties for the pre-block", () => {
    for (const [preBlock, format] of [
      ["b:: 1\na:: 2\nB:: 3\n\nprose", "md"],
      ["#+TITLE:Book\n:PROPERTIES:\n:owner: ann\n:END:\n#+tags: x", "org"],
    ] as [string, Format][]) {
      load("P", preBlock, ["body"], format);
      const seen = new Set<string>();
      const firstWins = pageProperties(preBlock, format).filter(([k]) => !seen.has(k.toLowerCase()) && !!seen.add(k.toLowerCase()));
      expect(readPageProperties("P")).toEqual(firstWins);
      resetStore();
    }
  });

  it("Org: lists drawer and no-space directives in file order, and edits them where they are", () => {
    load("Book", "#+TITLE:Book\n:PROPERTIES:\n:owner: ann\n:END:\nIntro", ["* body"], "org");
    expect(readPageProperties("Book")).toEqual([["title", "Book"], ["owner", "ann"]]);
    setPageProperty("Book", "owner", "bob");
    expect(pre("Book")).toBe("#+TITLE:Book\n:PROPERTIES:\n:owner: bob\n:END:\nIntro");
    setPageProperty("Book", "title", "Novel");
    expect(pre("Book")).toBe("#+title: Novel\n:PROPERTIES:\n:owner: bob\n:END:\nIntro");
    setPageProperty("Book", "owner", null);
    expect(readPageProperty("Book", "owner")).toBeNull();
    // Our removal emptied the drawer, so the drawer goes too.
    expect(pre("Book")).toBe("#+title: Novel\nIntro");
  });

  it("mixed pre-block + properties-only first root: the writer edits the root line it listed", () => {
    load("Mixed", "a:: 1", ["owner:: martin", "body"]);
    expect(readPageProperties("Mixed")).toEqual([["a", "1"], ["owner", "martin"]]);
    setPageProperty("Mixed", "owner", "ann");
    expect(pre("Mixed")).toBe("a:: 1");
    expect(doc.byId["Mixed-0"].raw).toBe("owner:: ann");
    expect(readPageProperty("Mixed", "owner")).toBe("ann");
    expect(readPageProperties("Mixed")).toEqual([["a", "1"], ["owner", "ann"]]);
  });

  it("removing the first header key keeps the rest of the header a header", () => {
    load("Sep", "a:: 1\n\nb:: 2\n\nIntro");
    setPageProperty("Sep", "a", null);
    expect(pre("Sep")).toBe("b:: 2\n\nIntro");
    expect(readPageProperties("Sep")).toEqual([["b", "2"]]);
  });

  it("removal clears every case-insensitive duplicate", () => {
    load("Dup", "owner:: a\nOWNER:: b\n\nIntro");
    setPageProperty("Dup", "owner", null);
    expect(readPageProperty("Dup", "owner")).toBeNull();
    expect(pre("Dup")).toBe("\nIntro");
  });
});

describe("block property Remove/replace is case-insensitive (G3 required neighbor)", () => {
  it("Remove clears every differently-cased duplicate, head and trailing", () => {
    load("B", null, ["title\nowner:: old\nOWNER:: newer\nbody\nOwner:: tail"]);
    expect(blockProperty("B-0", "owner")).toBe("old");
    setBlockProperty("B-0", "owner", null);
    expect(blockProperty("B-0", "owner")).toBeNull();
    expect(doc.byId["B-0"].raw).toBe("title\nbody");
  });

  it("replace keeps one line, in place, with the file's spelling", () => {
    load("B", null, ["title\nOwner:: old\nstatus:: x\nOWNER:: dup"]);
    setBlockProperty("B-0", "owner", "new");
    expect(doc.byId["B-0"].raw).toBe("title\nOwner:: new\nstatus:: x");
    expect(facetsOf(doc.byId["B-0"].raw, "md").properties).toEqual([["Owner", "new"], ["status", "x"]]);
  });

  it("a properties-only block (property on its first line) can be removed", () => {
    load("B", null, ["owner:: old\nOWNER:: newer"]);
    setBlockProperty("B-0", "owner", null);
    expect(blockProperty("B-0", "owner")).toBeNull();
  });

  it("Org drawer: removal clears every case-insensitive duplicate", () => {
    load("O", null, ["* title\n:PROPERTIES:\n:owner: a\n:OWNER: b\n:END:"], "org");
    setBlockProperty("O-0", "owner", null);
    expect(blockProperty("O-0", "owner")).toBeNull();
  });
});

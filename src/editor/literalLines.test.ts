// C3 L10/L13/L14/L16 (og c3s F4/F7): content inside a code/src/example block is
// never rewritten or moved by a property, id, planning, repeater or priority
// edit. NAMED OG DIVERGENCE: OG's `insert-property` hoists every line starting
// with SCHEDULED/DEADLINE and its writers scan code text (util/property.cljs
// 244-248); og keeps literal content byte-identical instead.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { facetsOf } from "../render/facets";
import { node as docNode, resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node } from "../document/model";
import { setBlockProperty, rawWithHeading } from "../document/edits/properties";
import { existingBlockId, rawWithBlockId } from "../document/edits/identity";
import { writeField } from "../sheet/fields";
import { setWorkflow } from "../ui";
import { joinProps, splitProps, isBuiltinHidden } from "./properties";
import { normalizePlanning } from "./planning";
import { rollRepeat, toggleTaskDone } from "./repeat";
import { literalBlockOfLine } from "./literalLines";

beforeAll(async () => {
  await initParser();
});
afterEach(() => resetStore());

function load(raw: string, format: "md" | "org" = "md") {
  const page: FeedPage = {
    name: "P", kind: "page", title: "P", preBlock: null, roots: ["a"],
    format, readOnly: false, guide: false,
  };
  const n: Node = { id: "a", raw, collapsed: false, parent: null, page: "P", children: [] };
  setWorkflow("todo");
  setDoc({ byId: { a: n }, pages: [page], feed: ["P"], loaded: true });
}

const CODE = "```cpp\nstd::cout << x;\n```";
const SCHED = "SCHEDULED: <2026-01-01 Thu>";

describe("literal lines are one lsdoc-backed answer", () => {
  it("marks code/src/example/math lines, delimiters included, and nothing else", () => {
    expect(literalBlockOfLine(`a\n${CODE}\nb`).map((n) => n !== -1)).toEqual([false, true, true, true, false]);
    expect(literalBlockOfLine("a\n#+BEGIN_SRC\nk:: v\n#+END_SRC").map((n) => n !== -1)).toEqual([false, true, true, true]);
    expect(literalBlockOfLine("a\n$$\nk:: v\n$$").map((n) => n !== -1)).toEqual([false, true, true, true]);
    // lsdoc: an unclosed fence is not code, so an `id::` after it is a property.
    expect(literalBlockOfLine("a\n```\ncode\nid:: q").every((n) => n === -1)).toBe(true);
    expect(facetsOf(rawWithBlockId("a\n```\ncode", "u-1", "md"), "md").properties).toContainEqual(["id", "u-1"]);
  });
});

describe("property and id writers leave code bytes alone (C3 L13)", () => {
  it("setBlockProperty on a whole-block fence whose 2nd line looks like a property", () => {
    load(CODE);
    setBlockProperty("a", "background-color", "red");
    const raw = docNode("a").raw;
    expect(raw.startsWith(`${CODE}\n`), `I-1: the code block stays byte-identical: ${JSON.stringify(raw)}`).toBe(true);
    expect(facetsOf(raw, "md").properties).toContainEqual(["background-color", "red"]);
  });

  it("the heading toggle writes its property outside the fence", () => {
    const raw = rawWithHeading(CODE, "md", true);
    expect(raw.startsWith(`${CODE}\n`), JSON.stringify(raw)).toBe(true);
    expect(facetsOf(raw, "md").properties).toContainEqual(["heading", "true"]);
  });

  it("an Org id drawer never hoists a SCHEDULED line out of a src block", () => {
    const raw = `a\n#+BEGIN_SRC\n${SCHED}\n#+END_SRC`;
    expect(rawWithBlockId(raw, "u-1", "org")).toBe(`a\n:PROPERTIES:\n:id: u-1\n:END:\n#+BEGIN_SRC\n${SCHED}\n#+END_SRC`);
    const planned = `a\n${SCHED}\n#+BEGIN_SRC\nDEADLINE: <2026-01-02 Fri>\n#+END_SRC`;
    expect(rawWithBlockId(planned, "u-1", "org")).toBe(
      `a\n${SCHED}\n:PROPERTIES:\n:id: u-1\n:END:\n#+BEGIN_SRC\nDEADLINE: <2026-01-02 Fri>\n#+END_SRC`
    );
  });

  it("the editor's Org hidden-drawer round trip keeps a src block intact (L14)", () => {
    const raw = `a\n:PROPERTIES:\n:id: u-1\n:END:\n#+BEGIN_SRC\n${SCHED}\n#+END_SRC`;
    const { visible, hidden } = splitProps(raw, isBuiltinHidden, "org");
    expect(joinProps(visible, hidden, "org")).toBe(raw);
  });

  it("a Markdown #+BEGIN_SRC `id::` line is code: not the block id, not hidden", () => {
    const raw = "a\n#+BEGIN_SRC\nid:: in-code\n#+END_SRC\nid:: real";
    expect(existingBlockId("a\n```\nid:: fake\n```", "md")).toBeNull();
    expect(existingBlockId(raw, "md")).toBe("real");
    const { visible, hidden } = splitProps(raw, isBuiltinHidden, "md");
    expect(hidden).toBe("id:: real");
    expect(joinProps(visible, hidden, "md")).toBe(raw);
  });
});

describe("planning, repeater and priority edits leave code bytes alone (C3 L10/L14/L16)", () => {
  it("a repeater roll advances only the real planning line", () => {
    const code = "```\nSCHEDULED: <2026-01-01 Thu +1d>\n```";
    expect(rollRepeat(`TODO x\nSCHEDULED: <2026-01-01 Thu +1d>\n${code}`, "todo")).toBe(
      `TODO x\nSCHEDULED: <2026-01-02 Fri +1d>\n${code}`
    );
    // A repeater only inside code does not make the task repeating.
    expect(toggleTaskDone(`TODO x\n${code}`, "todo")).toBe(`DONE x\n${code}`);
  });

  it("planning normalization never moves a line into a leading code fence", () => {
    const raw = `\`\`\`\ncode\n\`\`\`\n${SCHED}`;
    expect(normalizePlanning(raw, "md")).toBe(raw);
  });

  it("a sheet priority write does not break a fence opener", () => {
    load("```js\ncode\n```");
    writeField("a", "priority", "A");
    expect(doc.byId.a.raw).toBe("```js\ncode\n```");
  });
});

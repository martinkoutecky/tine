import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { setTypographyMode, typographyMode } from "../ui";

beforeAll(async () => { await initParser(); });
afterEach(() => { document.body.replaceChildren(); });

/** The opening text's lines as the user reads them. */
function shown(text: string, format: "md" | "org" = "md"): string[] {
  const host = document.createElement("div");
  document.body.append(host);
  render(() => <AstBody raw={text} format={format} opening />, host);
  const lines: string[] = [""];
  const walk = (node: Node) => {
    if (node.nodeName === "BR") lines.push("");
    else if (node.nodeType === Node.TEXT_NODE) {
      const [first, ...rest] = (node.textContent ?? "").split("\n");
      lines[lines.length - 1] += first;
      lines.push(...rest);
    }
    else node.childNodes.forEach(walk);
  };
  walk(host);
  return lines;
}

// D-20261119-PG, review da945bc19: opening text is shown as written. Task
// words, priorities and indented lines keep their text, and a later property
// group is its literal lines, never chips or nothing.
it("shows opening text as written, task words, priorities and later groups included", () => {
  const text = [
    "TODO [#A] first", "second line", "DONE third", "LATER [#C] fourth", "  indented", "\tTabbed",
    "", "page-level:: yes", "timestamp:: 1", "", "**bold** end",
  ].join("\n");
  expect(shown(text)).toEqual([
    "TODO [#A] first", "second line", "DONE third", "LATER [#C] fourth", "  indented", "\tTabbed",
    "", "page-level:: yes", "timestamp:: 1", "", "bold end",
  ]);
  expect(document.querySelector("strong, b")?.textContent).toBe("bold");
  expect(document.querySelector(".page-properties, .prop-key")).toBeNull();
});

it("keeps a group that directly follows a prose line on its own lines", () => {
  expect(shown("prose\na:: 1\nb:: 2\nmore")).toEqual(["prose", "a:: 1", "b:: 2", "more"]);
  expect(shown("é:: 1\n\nprosé\nb:: 2")).toEqual(["é:: 1", "", "prosé", "b:: 2"]);
});

it("parses the opening text once, whatever its length", () => {
  const lines = Array.from({ length: 200 }, (_, i) => (i % 3 ? `TODO line ${i}` : `k${i}:: v${i}`));
  const w = window as unknown as { __tineBench?: boolean; __tineParseStats?: { calls: number; hits: number; misses: number } };
  w.__tineBench = true;
  w.__tineParseStats = { calls: 0, hits: 0, misses: 0 };
  expect(shown(lines.join("\n"))).toEqual(lines);
  expect(w.__tineParseStats.misses).toBe(1);
});

// Review a55c0f03a: opening text renders through the block renderer, so every
// construct a block shows keeps its rendering; only property groups and a
// leading marker or priority become source text.
it("keeps a leading tab or space on a property line, and one break per CRLF", () => {
  expect(shown("prose\n\ta:: 1\n  b:: \n\u00e9t\u00e9:: x")).toEqual(["prose", "\ta:: 1", "  b:: ", "\u00e9t\u00e9:: x"]);
  expect(shown("prose\r\na:: 1\r\nb:: 2\r\nmore")).toEqual(["prose", "a:: 1", "b:: 2", "more"]);
});

it("keeps the spaces after a task word or priority, and a task word alone on its line", () => {
  expect(shown("TODO  [#A]   spaced")).toEqual(["TODO  [#A]   spaced"]);
  expect(shown("TODO\nnext")).toEqual(["TODO", "next"]);
  expect(shown("[#B]\nnext")).toEqual(["[#B]", "next"]);
  expect(shown("TODO\n\nnext")).toEqual(["TODO", "", "next"]);
});

it("renders a quote, fenced code and a source block as a block body does", () => {
  const text = ["> quoted", "", "```js", "let a = 1;", "```", "", "#+BEGIN_SRC python", "print(1)", "#+END_SRC", "", "k:: v"].join("\n");
  shown(text);
  expect(document.querySelector("blockquote")?.textContent).toContain("quoted");
  const code = [...document.querySelectorAll("pre, code")].map((n) => n.textContent).join("\n");
  expect(code).toContain("let a = 1;");
  expect(code).toContain("print(1)");
  expect(document.querySelector(".source-text")?.textContent).toBe("k:: v");
});

it("renders an Org source block in Org opening text", () => {
  shown(["prose", "#+BEGIN_SRC sh", "echo hi", "#+END_SRC"].join("\n"), "org");
  expect([...document.querySelectorAll("pre, code")].map((n) => n.textContent).join("\n")).toContain("echo hi");
  expect(document.body.textContent).toContain("prose");
});

it("does not apply render typography to a property group", () => {
  const before = typographyMode();
  setTypographyMode("render");
  try {
    expect(shown("a -> b\nk:: a -> b -- c")).toEqual(["a \u2192 b", "k:: a -> b -- c"]);
  } finally { setTypographyMode(before); }
});

// Review a55c0f03a: group spans are translated with one cursor per buffer, so
// the UTF-8 work grows with the text, not with groups times text.
it("encodes the opening text a linear number of times in its groups", () => {
  const encoded = (groups: number) => {
    const text = Array.from({ length: groups }, (_, i) => `k${i}:: v`).join("\n\n");
    const encode = TextEncoder.prototype.encode;
    let chars = 0;
    TextEncoder.prototype.encode = function (input?: string) { chars += input?.length ?? 0; return encode.call(this, input); };
    try { shown(text); } finally { TextEncoder.prototype.encode = encode; }
    document.body.replaceChildren();
    return { chars, length: text.length };
  };
  const small = encoded(32), large = encoded(256);
  expect(large.chars / large.length).toBeLessThan(2 * (small.chars / small.length) + 4);
});

it("shows a #+key: value line as written, as it does a key:: value group", () => {
  expect(shown("prose\n#+b: [[X]]\nmore")).toEqual(["prose", "#+b: [[X]]", "more"]);
});

// Review 716a8eb3b: by construction, every source range the renderer does not
// render itself (property nodes at any depth, separators, a first line's task
// word or priority, leading blank lines, query macros) shows as that source,
// once, with one line break per physical line end. For these shapes the
// rendered lines are exactly the source lines.
const exactShapes: [string, "md" | "org"][] = [
  ["TODO k:: v", "md"], ["[#A] k:: v", "md"], ["TODO [#A] k:: v", "md"], ["TODO k::", "md"], ["TODO #+k: v", "md"],
  ["TODO\nk:: v", "md"], ["DONE [#B]  k:: \nprose", "md"],
  ["prose\r\n\r\nk:: v", "md"], ["a:: 1\r\n\r\nb:: 2", "md"], ["k:: v\r\n\r\nprose", "md"], ["k:: v\r\n\r\n", "md"],
  ["prose\r\n\r\n#+k: v", "md"], ["prose\r\n\r\n#+k: v", "org"], ["#+a: 1\r\n\r\n#+b: 2\r\nprose", "org"],
  ["prose\n \nk:: v", "md"], ["prose\n  \nk:: v", "md"], ["prose\n\t\nk:: v", "md"],
  ["  \nprose\nk:: v", "md"], ["\n\nprose\nk:: v", "md"],
  ["{{query (task TODO)}}", "md"], ["{{QUERY x}}", "md"], ["{{tine-query x}}", "md"], ["{{Tine-Query x}}", "md"],
  ["prose {{query x}} more", "md"], ["prose {{query x}} more", "org"], ["k:: v\n{{tine-query (page p)}}", "md"],
];
it.each(exactShapes)("shows %j (%s) as its source lines", (text, format) => {
  expect(shown(text, format)).toEqual(text.split(/\r?\n/));
});

it.each([
  ["#+BEGIN_QUOTE\n#+k: v\n#+END_QUOTE", "md"], ["#+BEGIN_QUOTE\n#+k: v\n#+END_QUOTE", "org"],
  ["#+BEGIN_NOTE\n#+k: v\n#+END_NOTE", "md"], ["#+BEGIN_NOTE\n#+k: v\n#+END_NOTE", "org"],
] as const)("shows a #+key: value line inside a container as its source: %j (%s)", (text, format) => {
  shown(text, format);
  expect([...document.querySelectorAll(".source-text")].map((n) => n.textContent)).toContain("#+k: v\n");
});

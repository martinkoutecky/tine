import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { OpeningText } from "./openingText";
import { initParser } from "./parse";

beforeAll(async () => { await initParser(); });
afterEach(() => { document.body.replaceChildren(); });

/** The opening text's lines as the user reads them. */
function shown(text: string, format: "md" | "org" = "md"): string[] {
  const host = document.createElement("div");
  document.body.append(host);
  render(() => <OpeningText text={text} format={format} />, host);
  const lines: string[] = [""];
  const walk = (node: Node) => {
    if (node.nodeName === "BR") lines.push("");
    else if (node.nodeType === Node.TEXT_NODE) lines[lines.length - 1] += node.textContent;
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

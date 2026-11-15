import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { InlineText, PageRef } from "./inline";
import { initParser } from "./parse";
import { setGraphMeta } from "../graphSession";
import type { GraphMeta } from "../types";

beforeAll(async () => { await initParser(); });
afterEach(() => { setGraphMeta(null); document.body.replaceChildren(); });

/** What the user sees: each page-reference anchor's text, brackets included. */
function seen(text: string, format: "org" | "md", brackets: boolean): string[] {
  setGraphMeta({ show_brackets: brackets } as GraphMeta);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text={text} format={format} />, host);
  try { return [...host.querySelectorAll("a.page-ref, a.tag")].map(a => a.textContent ?? ""); } finally { dispose(); }
}

// GH #966; OG block.cljs `page-reference` draws the brackets around the label of
// a labelled page link, not only around a bare one.
const REPORTED = "A [[git]] tool for [[Emacs][my favorite editor]]";

it("GH #966: an Org labelled page link shows brackets like the plain one", () => {
  expect(seen(REPORTED, "org", true)).toEqual(["[[git]]", "[[my favorite editor]]"]);
});

it("GH #966: with Show brackets off neither Org link shows brackets", () => {
  expect(seen(REPORTED, "org", false)).toEqual(["git", "my favorite editor"]);
});

it("GH #966: the Markdown labelled page link follows the same setting", () => {
  const md = "A [[git]] tool for [my favorite editor]([[Emacs]])";
  expect(seen(md, "md", true)).toEqual(["[[git]]", "[[my favorite editor]]"]);
  expect(seen(md, "md", false)).toEqual(["git", "my favorite editor"]);
});

it("GH #966: the Markdown dotless target form [label](Page) is a page reference too, as in OG", () => {
  const md = "see [my favorite editor](Emacs)";
  expect(seen(md, "md", true)).toEqual(["[[my favorite editor]]"]);
  expect(seen(md, "md", false)).toEqual(["my favorite editor"]);
});

it("GH #966: dotted paths, anchors and block refs stay ordinary links, not page references", () => {
  expect(seen("[x](notes.txt) [y](#anchor) [z](*Headline) [w](((6574a9e1-0000-4000-8000-000000000000)))", "md", true)).toEqual([]);
});

it("GH #966: a #tag and an explicit alias label of a property value keep no brackets", () => {
  setGraphMeta({ show_brackets: true } as GraphMeta);
  expect(seen("#project", "org", true)).toEqual(["#project"]);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <PageRef name="Target" alias="Target" />, host);
  try { expect(host.querySelector("a")?.textContent).toBe("Target"); } finally { dispose(); }
});

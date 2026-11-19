import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { AstBody } from "../render/body";
import { initParser } from "../render/parse";
import { openingSegments } from "../editor/properties";
import { OpeningText } from "./OpeningText";
import { OPENING_DRAWER_CASES, shownLines } from "../tests/openingClickCases";

beforeAll(async () => { await initParser(); });
afterEach(() => { document.body.replaceChildren(); });

function mount(view: () => JSX.Element): HTMLElement {
  const host = document.createElement("div");
  document.body.append(host);
  render(view, host);
  return host;
}

/** Each piece of the opening row in order: a group's shown text, or a block piece's HTML. */
function pieces(text: string, format: "md" | "org" = "md"): { group: boolean; shown: string }[] {
  const host = mount(() => <OpeningText raw={text} format={format} />);
  return [...host.children].filter((el) => !el.classList.contains("opening-blank")).map((el) => el.classList.contains("opening-group")
    ? { group: true, shown: el.textContent ?? "" }
    : { group: false, shown: el.innerHTML });
}

/** The HTML of `text` rendered as an ordinary block body, without source
 *  coordinates: opening text is read with an empty title, so a line end's
 *  span differs from a block title's while what is drawn is the same. */
function asBlockBody(text: string, format: "md" | "org" = "md"): string {
  return drawn(mount(() => <AstBody raw={text} format={format} />).innerHTML);
}
const drawn = (html: string) => html.replace(/ data-s(?:o|e|ce)="\d+"/g, "");

/** The lines opening text shows: its source lines, except that a Markdown
 *  hard break's trailing spaces and tabs are syntax, not drawn (as on every
 *  body line after a block's title). */
function sourceLines(text: string, format: "md" | "org"): string[] {
  const lines = text.split("\n");
  return format === "md" ? lines.map((l, i) => (i < lines.length - 1 && HARD_BREAK.has(l) ? l.trimEnd() : l)) : lines;
}
const HARD_BREAK = new Set(["TODO \t", "TODO  "]);


// D-20261119-PG, redesign B: the parser names opening text's top-level
// property groups; each shows as its exact source, one line per source line
// (a Windows line end is one), indentation and any text kept, no markup.
it.each([
  ["prose\n\nk:: v\n\nmore", "md", ["k:: v"]],
  ["prose\nk:: v", "md", ["k:: v"]],
  ["a:: 1\n\nb:: 2", "md", ["a:: 1", "b:: 2"]],
  ["prose\r\n\r\nk:: v\r\nj:: w\r\n\r\nmore", "md", ["k:: v\nj:: w"]],
  ["prose\n\n  k:: v\n\tj:: w", "md", ["  k:: v\n\tj:: w"]],
  ["prose\n\nstatus:: TODO [#A] **x** [[P]]", "md", ["status:: TODO [#A] **x** [[P]]"]],
  ["prose\n#+k: [[X]]\nmore", "md", ["#+k: [[X]]"]],
  ["prose\n#+k: v", "org", ["#+k: v"]],
  ["prose\n\nkey:: été 日本\n\nmore", "md", ["key:: été 日本"]],
  ["TODO k:: v", "md", ["TODO k:: v"]],
  ["[#A] k:: v", "md", ["[#A] k:: v"]],
  ["TODO [#A] k:: v\r\nprose", "md", ["TODO [#A] k:: v"]],
  ["TODO #+k: v", "md", ["TODO #+k: v"]],
] as const)("shows the group of %j (%s) as its source", (text, format, groups) => {
  expect(pieces(text, format).filter((p) => p.group).map((p) => p.shown)).toEqual(groups);
  const fromParser = openingSegments(text, format).filter((s) => s.group).map((s) => s.text);
  expect(fromParser.map((g) => g.replace(/\r?\n$/, "").replace(/\r\n/g, "\n"))).toEqual(groups);
  expect(fromParser.every((g) => text.includes(g))).toBe(true);
});

// Everything else is block text: it renders exactly as the same text does
// as a block body, queries, macros, containers and hidden directives included.
it.each([
  ["> q\n#+k: v", "md"], ["> q\n#+k: v", "org"],
  ["#+BEGIN_QUOTE\n#+k: v\n#+END_QUOTE", "md"], ["#+BEGIN_NOTE\n#+k: v\n#+END_NOTE", "org"],
  ["{{query (task TODO)}}", "md"], ["prose {{tine-query x}} more", "org"],
  ["{{embed [[X]]}}", "md"], ["{{video https://example.com/v.mp4}}", "md"],
  ["**bold** [[Page]] #tag", "md"], ["line one\nline two", "md"],
] as const)("renders %j (%s), which has no top-level group, as a block body", (text, format) => {
  const shown = pieces(text, format);
  expect(shown.every((p) => !p.group)).toBe(true);
  expect(shown).toHaveLength(1);
  expect(drawn(shown[0].shown)).toBe(asBlockBody(text, format));
});

it("keeps groups and block pieces in source order, each piece a block body", () => {
  const text = "one {{query x}}\n\na:: 1\n\n> two\n\nb:: 2\nc:: 3\n\nthree";
  const shown = pieces(text);
  expect(shown.map((p) => p.group)).toEqual([false, true, false, true, false]);
  expect(shown.filter((p) => p.group).map((p) => p.shown)).toEqual(["a:: 1", "b:: 2\nc:: 3"]);
  const blocks = ["one {{query x}}", "> two", "three"];
  expect(shown.filter((p) => !p.group).map((p) => drawn(p.shown))).toEqual(blocks.map((b) => {
    document.body.replaceChildren();
    return asBlockBody(b);
  }));
});

it.each([
  ["prose\n\nk:: v\n\n\nmore", ["text", "blank", "group", "blank", "blank", "text"]],
  ["a:: 1\r\n\r\nb:: 2", ["group", "blank", "group"]],
  ["\n\nprose\nk:: v\n", ["blank", "blank", "text", "group"]],
  ["prose\n\n\n#+k: v", ["text", "blank", "blank", "group"]],
] as const)("shows each blank source line of %j between pieces", (text, kinds) => {
  const host = mount(() => <OpeningText raw={text} format="md" />);
  expect([...host.children].map((el) => el.className.match(/opening-(group|text|blank)/)![1])).toEqual(kinds);
});

// Ruling (b) on D-20261119-PG: a task word or priority the block parse lifts
// off a piece's first line prints as its exact source, never as chrome.
it.each([
  ["TODO Prose [[Target]]", "md", ["TODO Prose [[Target]]"]],
  ["[#A] Prose", "md", ["[#A] Prose"]],
  ["TODO [#A] Prose [[Target]]", "md", ["TODO [#A] Prose [[Target]]"]],
  ["DONE  done it", "md", ["DONE  done it"]],
  ["LATER [#C] later", "md", ["LATER [#C] later"]],
  ["NOW now", "md", ["NOW now"]],
  ["TODO", "md", ["TODO"]],
  ["TODO\nnext line", "md", ["TODO", "next line"]],
  ["TODO Org prose", "org", ["TODO Org prose"]],
  ["DONE [#B] Org prose", "org", ["DONE [#B] Org prose"]],
  ["a:: 1\n\nNOW after a group", "md", ["a:: 1", "", "NOW after a group"]],
  // Review 28d081755 F2: a lead's own spaces and tabs stay exact at a piece's end.
  ["TODO \t", "md", ["TODO \t"]],
  ["[#A]  ", "md", ["[#A]  "]],
  ["TODO [#B]\t", "org", ["TODO [#B]\t"]],
  ["a:: 1\r\n\r\nNOW  \r\n", "md", ["a:: 1", "", "NOW  "]],
  // Review fe3946dc9 F2: the lead runs to the parser's body start, so a line
  // end the parser keeps is drawn once (no inserted break).
  ...(["md", "org"] as const).flatMap((format) => [
    "TODO \nNext", "TODO \t\nNext", "TODO  \nNext", "[#A] \nNext", "[#A]\t\nNext",
    "TODO [#A] \nNext", "TODO [#A]\t \nNext", "TODO \n\nNext", "TODO [#B]\t\n\n\nNext",
    "DONE x\nNext", "TODO \n  indented",
  ].map((text) => [text, format, sourceLines(text, format)] as const)),
  // Review e852e1b58 F2: a drawer after the task word or priority.
  ...OPENING_DRAWER_CASES,
] as const)("shows the task word or priority of %j (%s) as text", (text, format, lines) => {
  const host = mount(() => <OpeningText raw={text} format={format} />);
  const shown = shownLines(host);
  expect(shown).toEqual(lines);
  expect(host.querySelector(".block-marker, .block-priority, input[type=checkbox]")).toBeNull();
});

// One parser answer per text, then one block parse per piece: the UTF-8 work
// grows with the text, not with pieces times text.
it("encodes opening text a linear number of times in its pieces", () => {
  const encoded = (pairs: number) => {
    const text = Array.from({ length: pairs }, (_, i) => `prose ${i}\n\nk${i}:: v`).join("\n\n");
    const proto = TextEncoder.prototype;
    const encode = proto.encode, encodeInto = proto.encodeInto;
    let chars = 0;
    proto.encode = function (input?: string) { chars += input?.length ?? 0; return encode.call(this, input); };
    proto.encodeInto = function (input: string, dest: Uint8Array) { chars += input.length; return encodeInto.call(this, input, dest); };
    try { mount(() => <OpeningText raw={text} format="md" />); } finally { proto.encode = encode; proto.encodeInto = encodeInto; }
    document.body.replaceChildren();
    return chars / text.length;
  };
  const small = encoded(32), large = encoded(256);
  expect(large).toBeLessThan(small * 1.25 + 1);
});

// Review e852e1b58 F1 (PC-3): a body's planning lines are found from one
// UTF-8 copy of it, shared by all its inline flows, so an opening of many
// quotes and prose lines with SCHEDULED:/DEADLINE: costs its length, not
// flows times length.
it.each([
  ["md", (i: number) => `> quote ${i}\n\nprose ${i}\nSCHEDULED: <2026-11-19 Thu>\nDEADLINE: <2026-11-20 Fri>`],
  ["org", (i: number) => `#+BEGIN_QUOTE\nquote ${i}\n#+END_QUOTE\nprose ${i}\nSCHEDULED: <2026-11-19 Thu>\nDEADLINE: <2026-11-20 Fri>`],
] as const)("encodes a %s opening of quotes and planning lines a linear number of times", (format, unit) => {
  const encoded = (units: number) => {
    const text = Array.from({ length: units }, (_, i) => unit(i)).join("\n");
    const proto = TextEncoder.prototype;
    const encode = proto.encode, encodeInto = proto.encodeInto;
    let chars = 0;
    proto.encode = function (input?: string) { chars += input?.length ?? 0; return encode.call(this, input); };
    proto.encodeInto = function (input: string, dest: Uint8Array) { chars += input.length; return encodeInto.call(this, input, dest); };
    try { mount(() => <OpeningText raw={text} format={format} />); } finally { proto.encode = encode; proto.encodeInto = encodeInto; }
    document.body.replaceChildren();
    return chars / text.length;
  };
  const small = encoded(32), large = encoded(64);
  expect(large).toBeLessThan(small * 1.25 + 1);
});

// Review 28d081755 F2 (PC-3): splitting a piece from its blank edge lines is
// one scan, so interior whitespace costs its length once, not its square (a
// regex trim took 1.6 s at 32,000 spaces). Eight times the text may cost about
// eight times as much (the block body itself grows with blank lines), never 64.
it.each([" ", "\t", "\n", "\r\n"])("splits a piece around 32,000 interior %j in linear time", (ws) => {
  const time = (chars: number) => {
    const text = `a${ws.repeat(chars / ws.length)}b`;
    const started = performance.now();
    const host = mount(() => <OpeningText raw={text} format="md" />);
    const elapsed = performance.now() - started;
    expect(host.textContent).toContain("b");
    document.body.replaceChildren();
    return elapsed;
  };
  time(4_000);
  const small = time(4_000), large = time(32_000);
  expect(large).toBeLessThan(12 * small + 50);
});

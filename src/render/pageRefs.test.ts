import { afterEach, beforeAll, expect, it } from "vitest";
import { initParser } from "./parse";
import { block_reference_names, decode_page_name, encode_page_name } from "./wasm/lsdoc_wasm.js";
import { blockRefsInText, linkTargetName, setFileNameFormat } from "./pageRefs";

beforeAll(() => initParser());
afterEach(() => setFileNameFormat(undefined));

// The window's page references are the reference evidence's own walk, compiled
// into wasm (`block_reference_names`, CM step 2 batch 6); these pin that it is
// the evidence's reading, not a second one.
it("reads a block's references off the evidence walk", () => {
  // `#[[Three x]]` is a tag and, inside it, a link: two occurrences.
  expect(block_reference_names("a [[One]] #Two #[[Three x]] **[[Four]]**", false)).toEqual(["One", "Two", "Three x", "Three x", "Four"]);
  expect(block_reference_names("x\ntags:: Five, [[Six]]", false)).toEqual(["Five", "Six"]);
  expect(block_reference_names("x\ntags:: \"Seven, Eight\"", false)).toEqual([]);
  expect(block_reference_names("{{embed [[Nine]]}}", false)).toEqual(["Nine"]);
});

it("treats code and prose as literal text", () => {
  expect(block_reference_names("`[[One]]` and educate #", false)).toEqual([]);
  expect(block_reference_names("```\n[[One]] #Two\n```", false)).toEqual([]);
});

it("splits fullwidth-comma tags like OG sep-by-comma", () => {
  expect(block_reference_names("x\ntags:: Old，Other", false)).toEqual(["Old", "Other"]);
});

it("names an Org file link's page with the graph's file-name format (UI-CM-ORG-FILE-LINK-LEGACY-FORMAT)", () => {
  // Legacy, the default: the store names Ten___Child.org "Ten___Child" and a.b.org "a/b".
  for (const legacy of [undefined, "legacy"]) {
    setFileNameFormat(legacy);
    expect(linkTargetName("file", "./Ten___Child.org", "ten", true)).toBe("Ten___Child");
    expect(linkTargetName("file", "./a.b.org", "ab", true)).toBe("a/b");
  }
  setFileNameFormat("triple-lowbar");
  expect(linkTargetName("file", "./Ten___Child.org", "ten", true)).toBe("Ten/Child");
  expect(linkTargetName("file", "./a.b.org", "ab", true)).toBe("a.b");
  expect(linkTargetName("file", "../pages/Ten___Child.org", "ten", true)).toBe("Ten/Child");
});

it("decodes percent-escaped Org file names", () => {
  expect(linkTargetName("file", "../pages/A%3AB.org", "title", true)).toBe("A:B");
});

it("shares native filename round trips in both naming formats", () => {
  for (const legacy of [true, false]) {
    for (const title of ["a/b", "a___b", "a_/b", "%2F", "a:b", "CON", "LPT³", "trailing dot.", "trailing space ", "Release 1.0", "é/字"]) {
      expect(decode_page_name(encode_page_name(title, legacy), legacy)).toBe(title);
    }
    expect(decode_page_name("A%3AB%ZZ", legacy)).toBe("A:B%ZZ");
  }
  expect(decode_page_name("Parent.Child", true)).toBe("Parent/Child");
  expect(decode_page_name("Parent___Child", false)).toBe("Parent/Child");
  setFileNameFormat("triple-lowbar");
  expect(linkTargetName("file", "../pages/A%5F%5F%5FB___Child%25.org", "title", true)).toBe("A___B/Child%");
});

it("reads block references in the text, its property values and macro arguments, not in code", () => {
  expect(blockRefsInText("see ((a1)) and `((b2))`\nrel:: ((c3))", "md")).toEqual(["a1", "c3"]);
  expect(blockRefsInText("{{embed ((d4))}}", "md")).toEqual(["d4"]);
});

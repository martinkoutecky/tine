import { afterEach, beforeAll, expect, it } from "vitest";
import { initParser } from "./parse";
import { decode_page_name, encode_page_name } from "./wasm/lsdoc_wasm.js";
import { pageRefsInText, setFileNameFormat } from "./pageRefs";

beforeAll(() => initParser());
afterEach(() => setFileNameFormat(undefined));

it("reads the references the backend rename rewrites off the lsdoc parse", () => {
  expect(pageRefsInText("a [[One]] #Two #[[Three x]] **[[Four]]**", "md")).toEqual(["One", "Two", "Three x", "Four"]);
  expect(pageRefsInText("x\ntags:: Five, [[Six]]", "md")).toEqual(["Six", "Five"]);
  expect(pageRefsInText("x\ntags:: \"Seven, Eight\"", "md")).toEqual([]);
  expect(pageRefsInText("{{embed [[Nine]]}}", "md")).toEqual(["Nine"]);
  setFileNameFormat("triple-lowbar");
  expect(pageRefsInText("see [[file:../pages/Ten___Child.org][ten]]", "org")).toContain("Ten/Child");
});

it("names an Org file link's page with the graph's file-name format (UI-CM-ORG-FILE-LINK-LEGACY-FORMAT)", () => {
  // Legacy, the default: the store names Ten___Child.org "Ten___Child" and a.b.org "a/b".
  for (const legacy of [undefined, "legacy"]) {
    setFileNameFormat(legacy);
    expect(pageRefsInText("see [[file:./Ten___Child.org][ten]] and [[file:./a.b.org][ab]]", "org")).toEqual(["Ten___Child", "a/b"]);
  }
  setFileNameFormat("triple-lowbar");
  expect(pageRefsInText("see [[file:./Ten___Child.org][ten]] and [[file:./a.b.org][ab]]", "org")).toEqual(["Ten/Child", "a.b"]);
});

it("treats code and prose as literal text", () => {
  expect(pageRefsInText("`[[One]]` and educate #", "md")).toEqual([]);
  expect(pageRefsInText("```\n[[One]] #Two\n```", "md")).toEqual([]);
});

it("splits fullwidth-comma tags like OG sep-by-comma", () => {
  expect(pageRefsInText("x\ntags:: Old，Other", "md")).toEqual(["Old", "Other"]);
});

it("decodes percent-escaped Org file names", () => {
  expect(pageRefsInText("see [[file:../pages/A%3AB.org][title]]", "org")).toContain("A:B");
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
  expect(pageRefsInText("[[file:../pages/A%5F%5F%5FB___Child%25.org][title]]", "org")).toContain("A___B/Child%");
});

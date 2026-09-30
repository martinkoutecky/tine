import { beforeAll, expect, it } from "vitest";
import { initParser } from "./parse";
import { pageRefsInText } from "./pageRefs";

beforeAll(() => initParser());

it("reads the references the backend rename rewrites off the lsdoc parse", () => {
  expect(pageRefsInText("a [[One]] #Two #[[Three x]] **[[Four]]**", "md")).toEqual(["One", "Two", "Three x", "Four"]);
  expect(pageRefsInText("x\ntags:: Five, [[Six]]", "md")).toEqual(["Six", "Five"]);
  expect(pageRefsInText("x\ntags:: \"Seven, Eight\"", "md")).toEqual([]);
  expect(pageRefsInText("{{embed [[Nine]]}}", "md")).toEqual(["Nine"]);
  expect(pageRefsInText("see [[file:../pages/Ten___Child.org][ten]]", "org")).toContain("Ten/Child");
});

it("treats code and prose as literal text", () => {
  expect(pageRefsInText("`[[One]]` and educate #", "md")).toEqual([]);
  expect(pageRefsInText("```\n[[One]] #Two\n```", "md")).toEqual([]);
});

it("splits fullwidth-comma tags like OG sep-by-comma", () => {
  expect(pageRefsInText("x\ntags:: Old，Other", "md")).toEqual(["Old", "Other"]);
});

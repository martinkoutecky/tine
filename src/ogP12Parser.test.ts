import { blockRegions, editBlock } from "./render/parse";
import { expect, it } from "vitest";
import { readPropertyValue, upsertPropertyLine, pagePropertyEntries } from "./editor/properties";
import { isPropertyLine } from "./render/block";
import { propertyOccurrences } from "./sheet/renameField";
import { pageRefsInText } from "./render/pageRefs";

it("D05 property editing and hiding use accepted grammar and source spans", () => {
  expect(isPropertyLine("foo::bar")).toBe(false);
  expect(isPropertyLine("klíč:: hodnota")).toBe(true);
  expect(readPropertyValue("```\nfoo:: ghost\n```\nfoo:: real", "foo")).toBe("real");
  expect(upsertPropertyLine("```\nfoo:: ghost\n```\nfoo:: real", "foo", "new")).toBe("```\nfoo:: ghost\n```\nfoo:: new");
  expect(propertyOccurrences("row\n  klíč:: hodnota", "md").map(p => p.key)).toEqual(["klíč"]);
});
it("D06 page aliases exclude parser-owned Org literals", () => {
  expect(pagePropertyEntries("#+BEGIN_SRC\n#+ALIAS: Ghost\n#+END_SRC", "org")).toEqual([]);
});
it("D11 page candidates exclude unlabeled assets and include nested links", () => {
  expect(pageRefsInText("[[file:../assets/paper.pdf]]", "org")).toEqual([]);
  expect(pageRefsInText("[[Outer [[Inner]]]]", "md").sort()).toEqual(["Inner", "Outer [[Inner]]"].sort());
});

it("D05 refilling an accepted empty property preserves readable syntax", () => {
  const raw = "Query\ntine.group-field:: ";
  const next = editBlock(raw, "md", { kind: "property", key: "tine.group-field", value: "prop:status" });
  expect(blockRegions(next).properties.find(p => p.key === "tine.group-field")?.value).toBe("prop:status");
});

it("D11 rename candidates keep the explicit-reference and bare-tags policy for aliases", () => {
  expect(pageRefsInText("x\nalias:: Alias", "md")).toEqual([]);
  expect(pageRefsInText("x\nalias:: [[Alias]]", "md")).toEqual(["Alias"]);
});

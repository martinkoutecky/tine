import { blockRegions, editBlock } from "./render/parse";
import { expect, it } from "vitest";
import { readPropertyValue, pagePropertyEntries } from "./editor/properties";
import { isPropertyLine } from "./render/block";
import { propertyOccurrences } from "./sheet/renameField";
import { block_reference_names } from "./render/wasm/lsdoc_wasm.js";

it("D05 property editing and hiding use accepted grammar and source spans", () => {
  expect(isPropertyLine("foo::bar")).toBe(false);
  expect(isPropertyLine("klíč:: hodnota")).toBe(true);
  expect(readPropertyValue("```\nfoo:: ghost\n```\nfoo:: real", "foo")).toBe("real");
  expect(editBlock("```\nfoo:: ghost\n```\nfoo:: real", "md", { kind: "property", key: "foo", value: "new" })).toBe("```\nfoo:: ghost\n```\nfoo:: new");
  expect(propertyOccurrences("row\n  klíč:: hodnota", "md").map(p => p.key)).toEqual(["klíč"]);
});
it("D06 page aliases exclude parser-owned Org literals", () => {
  expect(pagePropertyEntries("#+BEGIN_SRC\n#+ALIAS: Ghost\n#+END_SRC", "org")).toEqual([]);
});
it("D11 page candidates exclude unlabeled assets and include nested links", () => {
  expect(block_reference_names("[[file:../assets/paper.pdf]]", true)).toEqual([]);
  expect(block_reference_names("[[Outer [[Inner]]]]", false).sort()).toEqual(["Inner", "Outer [[Inner]]"].sort());
});

it("D05 refilling an accepted empty property preserves readable syntax", () => {
  const raw = "Query\ntine.group-field:: ";
  const next = editBlock(raw, "md", { kind: "property", key: "tine.group-field", value: "prop:status" });
  expect(blockRegions(next).properties.find(p => p.key === "tine.group-field")?.value).toBe("prop:status");
});

// D11's alias policy for rename candidates (a bare `alias::` member is not
// rewritten, an explicit `[[Alias]]` is) moved with the question to the rename
// itself: `crates/tine-graph-features/tests/rename_would_change.rs`
// `bare_alias_members_stay` (CM step 2 batch 6).

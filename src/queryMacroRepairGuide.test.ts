import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { queryMacroExtent } from "./editor/queryMacro";

it("Guide teaches repairing an unreadable titled query in the block text", () => {
  const guide = readFileSync("crates/tine-core/src/templates/queries.md", "utf8");
  const repair = guide.split("\n").find((line) => line.includes("Query source could not be parsed"));
  expect(repair).toBeDefined();
  expect(repair).toMatch(/edit the block text/i);
  const example = repair!.split("`").find((part) => part.startsWith("{{query"))!;
  expect(queryMacroExtent(example)?.argument).toBe('(page-property tags gptpro) {:title "gptpro"}');
});

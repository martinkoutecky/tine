import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import postcss from "postcss";

// Tine's stylesheets are flat. A rule that lands inside another rule's braces
// (a misplaced closing brace) is silently dropped by browsers without CSS
// nesting and scoped wrongly by those with it: the #881 spacing rules once
// sat inside `.query-empty { … }` and missed every populated query group.
describe("stylesheets have no rule nested in another rule", () => {
  for (const file of readdirSync("src/styles").filter(name => name.endsWith(".css"))) {
    it(file, () => {
      const nested: string[] = [];
      postcss.parse(readFileSync(`src/styles/${file}`, "utf8")).walkRules(rule => {
        if (rule.parent?.type === "rule") nested.push(`${rule.selector} inside ${(rule.parent as postcss.Rule).selector}`);
      });
      expect(nested).toEqual([]);
    });
  }
});

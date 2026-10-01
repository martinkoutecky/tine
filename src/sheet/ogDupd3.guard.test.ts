import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: sheet scalar/calendar policy lives in typed.ts; imitate formula/value.ts", () => {
  for (const path of ["src/sheet/aggregate.ts", "src/sheet/formula/value.ts", "src/sheet/formula/eval.ts"]) {
    const source = readFileSync(path, "utf8");
    expect(source, `I-12: ${path} must use typed.ts for sheet number/calendar policy`).not.toMatch(/parseFloat\(|Date\.UTC\(|function daysInMonth\(/);
    expect(source).toMatch(/from ["'](?:\.\/|\.\.\/)typed["']/);
  }
});

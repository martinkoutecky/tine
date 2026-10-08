// Regression guard for GH #256: shipped source must not contain regex
// lookbehind syntax. Safari/WKWebView older than 16.4 throws
// "SyntaxError: invalid group specifier name" while PARSING any eagerly loaded
// module that contains one; on macOS 12 with an un-updated WebKit that killed
// the whole app at startup (white screen) because src/mock.ts:65 shipped a
// lookbehind in the modulepreloaded bundle. Bundlers never transpile regex
// literals and no test environment runs that engine, so we grep the source.
// Test files are excluded: they run only under node and are never bundled
// (og's errorSwallow.guard.test.ts carries a lookbehind in a guard pattern).
import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const SRC = fileURLToPath(new URL(".", import.meta.url));

function* walk(dir: string): Generator<string> {
  for (const entry of readdirSync(dir)) {
    if (entry === "node_modules" || entry === ".git") continue;
    const p = join(dir, entry);
    const st = statSync(p);
    if (st.isDirectory()) yield* walk(p);
    else if (/\.(ts|tsx|mjs|js)$/.test(entry) && !/\.test\.(ts|tsx)$/.test(entry)) yield p;
  }
}

describe("WebKit compatibility (GH #256)", () => {
  it("source contains no regex lookbehind (unsupported by pre-16.4 Safari/WKWebView)", () => {
    const offenders: string[] = [];
    for (const file of walk(SRC)) {
      const text = readFileSync(file, "utf8");
      for (const [i, line] of text.split("\n").entries()) {
        if (/^\s*(\/\/|\*)/.test(line)) continue; // whole-line comments are harmless
        if (/\(\?<=|\(\?<!/.test(line)) offenders.push(`${relative(SRC, file)}:${i + 1}`);
      }
    }
    expect(offenders).toEqual([]);
  });
});

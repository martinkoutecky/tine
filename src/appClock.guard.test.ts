// Every frontend read of "now" goes through `appNow()` (src/journal.ts), never
// a bare `new Date()` (GH #607).
//
// The backend's time-zone rules decide which calendar day it is: journal
// membership, the journal feed's `as_of_day`, relative queries. A WebView can
// carry older zone rules than the OS — the Linux AppImage bundles its own ICU,
// which still applied Mexico City's abolished daylight saving time — and a
// frontend "today" read from `new Date()` then disagreed with the backend's for
// an hour every night. The journal feed refuses a response for a different day
// than the one it asked about, so journals stopped loading. `appNow()` carries
// the correction; a bare `new Date()` silently reintroduces the split.
//
// Exempt: the clock itself, and backends that have no native clock to defer to
// (the in-browser mock and the published export, where the browser IS the
// authority). A constructed date (`new Date(y, m, d)`, `new Date(ms)`) is not a
// read of "now" and is not matched.
import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const srcDir = path.dirname(fileURLToPath(import.meta.url));
const EXEMPT = new Set(["journal.ts", "mock.ts", "publishedBackend.ts"]);

function productFiles(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return productFiles(full);
    if (!/\.tsx?$/.test(entry.name) || /\.test\.tsx?$/.test(entry.name)) return [];
    return [full];
  });
}

export function bareNowSites(source: string): number[] {
  return source.split("\n").flatMap((line, index) => {
    const code = line.replace(/\/\/.*$/, "");
    if (/^\s*\*/.test(code)) return [];
    return /\bnew Date\(\s*\)/.test(code) ? [index + 1] : [];
  });
}

describe("app clock authority (GH #607)", () => {
  it("detects a bare read of now and ignores constructed dates and comments", () => {
    expect(bareNowSites("const d = new Date();")).toEqual([1]);
    expect(bareNowSites("const d = new Date(2026, 0, 1);\n// new Date()\nnew Date(ms)")).toEqual([]);
  });

  it("no product file reads the wall clock except through appNow()", () => {
    const offenders = productFiles(srcDir)
      .filter((file) => !EXEMPT.has(path.relative(srcDir, file)))
      .flatMap((file) =>
        bareNowSites(fs.readFileSync(file, "utf8")).map((line) => `${path.relative(srcDir, file)}:${line}`)
      );
    expect(
      offenders,
      "Read 'now' with appNow() from src/journal.ts (GH #607): the backend's zone rules are the calendar " +
        "authority, and a WebView's bundled ICU can disagree with them. Exemplar: src/carry.ts."
    ).toEqual([]);
  });
});

// A field the app inserts while it runs takes focus through `focusOnMount`
// (src/focusOnMount.ts), never the `autofocus` attribute (GH #619 c6).
//
// Chromium (WebView2 on Windows, Android's WebView) honours `autofocus` once
// per document, so the second query rename or condition value needed a click
// there while WebKitGTK on Linux looked fine; WebKit is not reliable either
// (GH #176). A new `autofocus` silently reintroduces the Windows-only defect.
import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const srcDir = path.dirname(fileURLToPath(import.meta.url));

function productFiles(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return productFiles(full);
    if (!/\.tsx$/.test(entry.name) || /\.test\.tsx$/.test(entry.name)) return [];
    return [full];
  });
}

export function autofocusSites(source: string): number[] {
  return source.split("\n").flatMap((line, index) => {
    const code = line.replace(/\/\/.*$/, "");
    return /(^|\s)autofocus(\s*=|\s*$|\s*\/?>)/.test(code) ? [index + 1] : [];
  });
}

describe("fields focus through focusOnMount (GH #619 c6)", () => {
  it("detects the attribute and ignores comments", () => {
    expect(autofocusSites("<input\n  autofocus\n/>")).toEqual([2]);
    expect(autofocusSites("<input autofocus={true} />")).toEqual([1]);
    // A component's own prop (CrossingNotice autoFocus) is not the DOM attribute.
    expect(autofocusSites("<CrossingNotice autoFocus={x} />")).toEqual([]);
    expect(autofocusSites("// autofocus is unreliable\nref={focusOnMount}")).toEqual([]);
  });

  it("no component uses the autofocus attribute", () => {
    const offenders = productFiles(srcDir).flatMap((file) =>
      autofocusSites(fs.readFileSync(file, "utf8")).map((line) => `${path.relative(srcDir, file)}:${line}`));
    expect(offenders, "Use ref={(input) => focusOnMount(input)} from src/focusOnMount.ts (GH #619 c6).").toEqual([]);
  });
});

import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the native outside-PDF command gates every shipped target before reading", () => {
  const source = readFileSync("src-tauri/src/external_pdf.rs", "utf8");
  const command = source.slice(source.indexOf("pub(crate) async fn read_external_pdf"));
  const branches = [...command.matchAll(/#\[cfg\(any\(([^]*?)\)\)\]\s*\{([^]*?)\n    \}/g)];
  const platforms = (text: string) => [...text.matchAll(/target_os = "([^"]+)"/g)].map(match => match[1]).sort();
  expect(branches).toHaveLength(2);
  expect(platforms(branches[0][1])).toEqual(["linux", "macos", "windows"]);
  expect(platforms(branches[1][1])).toEqual(["android", "ios"]);
  expect(branches[0][2]).toContain("off_ui(move || read_pdf");
  expect(branches[1][2]).toContain('Err("external-pdf-desktop-only"');
  expect(branches[1][2]).not.toMatch(/read_pdf|canonicalize|File::open/);
});

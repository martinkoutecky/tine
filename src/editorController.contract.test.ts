import { readdir, readFile } from "node:fs/promises";
import { dirname, extname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const srcDir = dirname(fileURLToPath(import.meta.url));
const thisTest = fileURLToPath(import.meta.url);
const controller = join(srcDir, "editorController.ts");

const forbiddenTokens = [
  "setEditingId",
  "setEditingOwner",
  "setCaretTarget",
  "setActiveSurface",
  "pendingFocusSurface",
];

async function sourceFiles(dir: string): Promise<string[]> {
  const entries = await readdir(dir, { withFileTypes: true });
  const files: string[] = [];
  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await sourceFiles(full)));
    } else if ([".ts", ".tsx"].includes(extname(entry.name))) {
      files.push(full);
    }
  }
  return files;
}

describe("editorController sole-writer contract", () => {
  it("keeps raw caret/focus writers private to the controller", async () => {
    const offenders: string[] = [];
    for (const file of await sourceFiles(srcDir)) {
      if (file === controller || file === thisTest) continue;
      const text = await readFile(file, "utf8");
      for (const token of forbiddenTokens) {
        if (text.includes(token)) offenders.push(`${file.replace(`${srcDir}/`, "")}: ${token}`);
      }
    }

    expect(
      offenders,
      [
        "ADR 0013 requires editorController to be the sole writer of caret/focus state.",
        "Use the controller intent API: startEditing, endEdit, noteSurfaceFocused, takeCaretFor, focusSurfaceFor, clearFocusSurface.",
      ].join(" ")
    ).toEqual([]);
  });
});

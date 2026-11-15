import { afterEach, describe, expect, it } from "vitest";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { testFilesObservingSetup } from "../scripts/lib/test-import-graph.mjs";

// The CM6 pass (vitest.cm6.config.ts) runs only the render tests that can observe
// what its setup overrides; these pin which files that derivation selects.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
let fixture: string | undefined;
afterEach(() => { if (fixture) rmSync(fixture, { recursive: true, force: true }); fixture = undefined; });

function project(files: Record<string, string>): string {
  fixture = mkdtempSync(path.join(tmpdir(), "cm6-pass-selection-"));
  for (const [name, source] of Object.entries(files)) {
    mkdirSync(path.dirname(path.join(fixture, name)), { recursive: true });
    writeFileSync(path.join(fixture, name), source);
  }
  return fixture;
}

describe("CM6 pass file selection", () => {
  it("selects exactly the tests whose runtime imports reach an overridden module, global or patched member", () => {
    const dir = project({
      "src/setup.ts": `import { vi } from "vitest";\nimport "./layout";\nvi.mock("./editor/switch", async () => ({}));\n(globalThis as { __forced?: boolean }).__forced = true;\n`,
      "src/layout.ts": "Range.prototype.measureRows = () => [];\n",
      "node_modules/viewpkg/dist/index.js": "export const view = range => range.measureRows();\n",
      "node_modules/otherpkg/index.js": "export const other = 1;\n",
      "src/editor/switch.ts": "export const on = () => false;\n",
      "src/editor/index.ts": `export { on } from "./switch";\n`,
      "src/Block.tsx": `import { on } from "./editor/switch";\nexport const Block = () => on();\n`,
      "src/Page.tsx": `import { Block } from "./Block.js";\nexport const Page = () => Block();\n`,
      "src/lazy.ts": `export const load = () => import("./Page");\n`,
      "src/dual.ts": "export const skip = () => (globalThis as { __forced?: boolean }).__forced;\n",
      "src/plain.ts": "export const plain = 1;\n",
      "src/types.ts": `import type { on } from "./editor/switch";\nexport type On = typeof on;\n`,
      "src/a.test.tsx": `import { Page } from "./Page";\n`,
      "src/b.test.tsx": `import { plain } from "./plain";\nimport type { On } from "./types";\n`,
      "src/c.test.tsx": `import { load } from "./lazy";\n`,
      "src/d.test.tsx": `import { skip } from "./dual";\n`,
      "src/e.test.tsx": `import { on } from "./editor";\n`,
      "src/f.test.tsx": `import type { on } from "./editor/switch";\nimport "./plain";\n`,
      "src/g.test.tsx": `import { view } from "viewpkg";\n`,
      "src/h.test.tsx": `import { other } from "otherpkg";\n`,
    });
    expect(testFilesObservingSetup({ root: dir, include: ["src/**/*.test.tsx"], setupFile: "src/setup.ts" }))
      .toEqual(["src/a.test.tsx", "src/c.test.tsx", "src/d.test.tsx", "src/e.test.tsx", "src/g.test.tsx"]);
  });

  it("runs every editor-mounting render test of this repository and skips the rest", () => {
    const config = readFileSync(path.join(root, "vitest.cm6.config.ts"), "utf8");
    expect(config).toContain(`const setupFile = "./src/tests/setup.cm6.ts"`);
    const selected = testFilesObservingSetup({ root, include: ["src/**/*.test.tsx"], setupFile: "./src/tests/setup.cm6.ts" });
    // Dual-engine files (useEditorKind), CM6-only defects the pass caught (census D5, D14,
    // the plain-paste textarea half), a Settings toggle for the switch itself, and files
    // that mount a CodeMirror view directly, which measures through the stubbed Range.
    for (const file of [
      "src/components/Block.commitExitParity.test.tsx",
      "src/components/Block.embedOneBullet.test.tsx",
      "src/components/Block.rtlArrows.test.tsx",
      "src/components/Block.plainPaste.test.tsx",
      "src/components/Block.referenceAuthoring.test.tsx",
      "src/components/Settings.cm6Beta.test.tsx",
      "src/tests/editorDriver.cm6.test.tsx",
      "src/components/MobileKeyboardToolbar.cm6CaretFollow.test.tsx",
    ]) expect(selected).toContain(file);
    // Pure policy and a dialog that never imports an editor.
    expect(selected).not.toContain("src/components/Block.contextMenu.test.tsx");
    expect(selected).not.toContain("src/components/AboutTab.test.tsx");
  });
});

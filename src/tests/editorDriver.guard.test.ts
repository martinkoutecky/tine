import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/**
 * Ratchet for the editor-neutral test driver (src/tests/editorDriver.ts, CM6
 * stage 1, D-20261025-BW): component tests reach the block editor only through
 * the driver, so an alternative editor can run the same suite. Files below
 * still touch a textarea directly and are being migrated; the list only
 * shrinks. A file whose textarea is NOT the block editor (a formula field, a
 * settings input) moves to OTHER_TEXTAREAS with its reason instead.
 */
const LEGACY = new Set<string>();

/** Test files whose textarea is a different control than the block editor. */
const OTHER_TEXTAREAS = new Map<string, string>([
  ["src/blockKeyboardCommands.test.tsx", "Standalone selected-text search fixture; mounted block-editor access uses the driver."],
  ["src/components/Block.mobileSelectionToolbar.test.tsx", "Standalone competing-focus textarea; native selection getter tests are textarea-kind guarded."],
  ["src/components/SheetBoard.test.tsx", "Standalone data-outside-editor focus fixture; block-editor access uses the driver."],
  ["src/components/QueryMacro.ir.test.tsx", "Standalone query-text-pane-input (QueryBuilder.tsx); block-editor focus uses the driver."],
  ["src/components/Block.marginComment.test.tsx", "Standalone comment-reply-draft (CommentParts.tsx); block-editor access uses the driver."],
  ["src/assetLanding.test.ts", "Inert AssetEditorToken identity fixture; no mounted editor or textarea state is accessed."],
  ["src/components/QueryCrossingNotice.test.tsx", "Standalone query-text-pane-input (QueryBuilder.tsx)."],
  ["src/components/QueryWorkspace.test.tsx", "Standalone query-text-pane-input (QueryBuilder.tsx)."],
  ["src/textareaPrimarySelection.test.tsx", "Standalone PRIMARY-selection fixtures; the block-editor placeholder assertion is textarea-kind guarded."],
  ["src/components/RefBlocks.orgBody.test.tsx", "Readonly export-preview payload (ExportModal.tsx)."],
  ["src/components/CssEditor.test.tsx", "Standalone custom CSS settings editor (CssEditor.tsx)."],
  ["src/components/DiagnosticsTab.test.tsx", "Readonly diagnostics report preview (DiagnosticsTab.tsx)."],
  ["src/components/ExportModal.memory.test.tsx", "Readonly export-preview payload (ExportModal.tsx)."],
  ["src/components/ExportModal.ogFront.test.tsx", "Readonly export-preview payload (ExportModal.tsx)."],
  ["src/components/ExportModal.qd6.test.tsx", "Readonly export-preview payload (ExportModal.tsx)."],
  ["src/components/ExportModal.test.tsx", "Readonly export-preview payload (ExportModal.tsx)."],
  ["src/components/FormulaEditor.test.tsx", "Standalone formula-editor-textarea (FormulaEditor.tsx)."],
  ["src/components/GraphVerification.test.tsx", "Graph report comparison input (GraphVerification.tsx)."],
  ["src/components/QueryTextPane.test.tsx", "Standalone query-text-pane-input (QueryBuilder.tsx)."],
]);

const DIRECT = /textarea\.block-editor|querySelector(?:All)?(?:<[^>]*>)?\(["']textarea["']\)|HTMLTextAreaElement/u;

describe("block-editor tests go through the editor driver", () => {
  const tests = readdirSync("src", { recursive: true, encoding: "utf8" })
    .filter((f) => /\.test\.tsx?$/u.test(f))
    .map((f) => `src/${f}`)
    .filter((f) => f !== "src/tests/editorDriver.guard.test.ts");
  const direct = tests.filter((f) => DIRECT.test(readFileSync(f, "utf8"))).sort();

  it("adds no new direct textarea access", () => {
    expect(direct.filter((f) => !LEGACY.has(f) && !OTHER_TEXTAREAS.has(f))).toEqual([]);
  });

  it("drops migrated files from the legacy list", () => {
    expect([...LEGACY].filter((f) => !direct.includes(f))).toEqual([]);
    expect([...OTHER_TEXTAREAS.keys()].filter((f) => !direct.includes(f))).toEqual([]);
  });
});

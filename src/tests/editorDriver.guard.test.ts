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
const LEGACY = new Set([
  "src/App.mobileEditingNavigation.test.tsx",
  "src/blockKeyboardCommands.test.tsx",
  "src/components/Block.assetOperation.test.tsx",
  "src/components/Block.assetPaste.test.tsx",
  "src/components/Block.clockInParagraph.test.tsx",
  "src/components/Block.codeBodyEdit.test.tsx",
  "src/components/Block.codeDelete.test.tsx",
  "src/components/Block.codeFenceEdit.test.tsx",
  "src/components/Block.datePickerCommit.test.tsx",
  "src/components/Block.deleteInput.test.tsx",
  "src/components/Block.embedExitUp.test.tsx",
  "src/components/Block.embedHierarchy.test.tsx",
  "src/components/Block.embedMoveNotice.test.tsx",
  "src/components/Block.embedStructural.test.tsx",
  "src/components/Block.imeComposition.test.tsx",
  "src/components/Block.marginComment.test.tsx",
  "src/components/Block.mixedCodeEdit.test.tsx",
  "src/components/Block.mobileCapture.test.tsx",
  "src/components/Block.mobileSelectionToolbar.test.tsx",
  "src/components/Block.multilinePaste.test.tsx",
  "src/components/Block.pasteAsLink.test.tsx",
  "src/components/Block.payloadPaste.test.tsx",
  "src/components/Block.referenceAuthoring.test.tsx",
  "src/components/Block.tabOwnership.test.tsx",
  "src/components/MarginColumn.test.tsx",
  "src/components/Page.batch11e.test.tsx",
  "src/components/Page.test.tsx",
  "src/components/Page.windowing.test.tsx",
  "src/components/QueryMacro.ir.test.tsx",
  "src/components/QueryMacro.test.tsx",
  "src/components/RenderedSelectionToolbar.test.tsx",
  "src/components/RightSidebar.r1.closeSafety.test.tsx",
  "src/components/Settings.codeDisplay.test.tsx",
  "src/components/SheetBoard.test.tsx",
  "src/components/SheetGrid.interact.test.tsx",
  "src/components/SheetTable.test.tsx",
  "src/components/landingOwnership.test.tsx",
  "src/editor/reattachProperties.test.tsx",
  "src/filedrop.assetBinding.test.tsx",
  "src/inpageReplace.dom.test.tsx",
  "src/keybindings.p1a1.globalCapture.test.tsx",
  "src/keystrokeScaling.guard.test.tsx",
  "src/render/PeekPopup.editInPreview.test.tsx",
  "src/render/calcCopy.test.tsx",
  "src/workspaceWindows.test.tsx",
]);

/** Test files whose textarea is a different control than the block editor. */
const OTHER_TEXTAREAS = new Map<string, string>([
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

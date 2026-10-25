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
  "src/assetLanding.test.ts",
  "src/blockKeyboardCommands.test.tsx",
  "src/components/Block.advancedCommandAutocomplete.test.tsx",
  "src/components/Block.arrowNavigation.test.tsx",
  "src/components/Block.assetOperation.test.tsx",
  "src/components/Block.assetPaste.test.tsx",
  "src/components/Block.autocompleteCloseOutside.test.tsx",
  "src/components/Block.boundaryHorizontal.test.tsx",
  "src/components/Block.calcBlur.test.tsx",
  "src/components/Block.carryReloadBlur.test.tsx",
  "src/components/Block.clickPastEnd.test.tsx",
  "src/components/Block.clockInParagraph.test.tsx",
  "src/components/Block.codeBodyEdit.test.tsx",
  "src/components/Block.codeCardClick.test.tsx",
  "src/components/Block.codeDelete.test.tsx",
  "src/components/Block.codeFenceAutocomplete.test.tsx",
  "src/components/Block.codeFenceEdit.test.tsx",
  "src/components/Block.codeFenceEnter.test.tsx",
  "src/components/Block.codeFenceScaffold.test.tsx",
  "src/components/Block.codePlanningBlur.test.tsx",
  "src/components/Block.copyEmbed.test.tsx",
  "src/components/Block.datePickerCommit.test.tsx",
  "src/components/Block.datePickerFocus.test.tsx",
  "src/components/Block.deleteInput.test.tsx",
  "src/components/Block.docModeEnter.test.tsx",
  "src/components/Block.embedExitUp.test.tsx",
  "src/components/Block.embedHierarchy.test.tsx",
  "src/components/Block.embedMoveNotice.test.tsx",
  "src/components/Block.embedStructural.test.tsx",
  "src/components/Block.enterLineBoundary.test.tsx",
  "src/components/Block.fullWidthCloser.test.tsx",
  "src/components/Block.headerRootToast.test.tsx",
  "src/components/Block.imeComposition.test.tsx",
  "src/components/Block.journalEmptyDay.test.tsx",
  "src/components/Block.listCheckbox.test.tsx",
  "src/components/Block.listTabBoundary.test.tsx",
  "src/components/Block.literalRegions.test.tsx",
  "src/components/Block.liveGrowthAnchor.test.tsx",
  "src/components/Block.marginComment.test.tsx",
  "src/components/Block.mathBlockEnter.test.tsx",
  "src/components/Block.mixedCodeEdit.test.tsx",
  "src/components/Block.mobileCapture.test.tsx",
  "src/components/Block.mobileSelectionToolbar.test.tsx",
  "src/components/Block.mouseDownEdit.test.tsx",
  "src/components/Block.moveSelection.test.tsx",
  "src/components/Block.multilinePaste.test.tsx",
  "src/components/Block.numberedList.test.tsx",
  "src/components/Block.pageEmbedCollapse.test.tsx",
  "src/components/Block.pageEmbedOccurrence.test.tsx",
  "src/components/Block.pagePropertyEnter.test.tsx",
  "src/components/Block.pasteAsLink.test.tsx",
  "src/components/Block.payloadPaste.test.tsx",
  "src/components/Block.pickerStates.test.tsx",
  "src/components/Block.pluginOwnership.test.tsx",
  "src/components/Block.propertyAutocomplete.test.tsx",
  "src/components/Block.referenceAuthoring.test.tsx",
  "src/components/Block.selectAll.test.tsx",
  "src/components/Block.tabClickedSelection.test.tsx",
  "src/components/Block.tabNoopSelection.test.tsx",
  "src/components/Block.tabOwnership.test.tsx",
  "src/components/Block.tagAutocomplete.test.tsx",
  "src/components/Block.taskSlash.test.tsx",
  "src/components/Block.templateVars.test.tsx",
  "src/components/Block.thatdaySlash.test.tsx",
  "src/components/Block.todaySlash.test.tsx",
  "src/components/Block.youtubeTimestampCommand.test.tsx",
  "src/components/CssEditor.test.tsx",
  "src/components/DiagnosticsTab.test.tsx",
  "src/components/ExportModal.memory.test.tsx",
  "src/components/ExportModal.ogFront.test.tsx",
  "src/components/ExportModal.qd6.test.tsx",
  "src/components/ExportModal.test.tsx",
  "src/components/FormulaEditor.test.tsx",
  "src/components/GraphVerification.test.tsx",
  "src/components/LiveRefGroup.identity.test.tsx",
  "src/components/LiveRefGroup.nav.test.tsx",
  "src/components/LiveRefGroup.pageProperties.test.tsx",
  "src/components/MarginColumn.test.tsx",
  "src/components/Page.batch11e.test.tsx",
  "src/components/Page.journalReferences.test.tsx",
  "src/components/Page.test.tsx",
  "src/components/Page.windowing.test.tsx",
  "src/components/QueryCrossingNotice.test.tsx",
  "src/components/QueryMacro.anchor.test.tsx",
  "src/components/QueryMacro.ir.test.tsx",
  "src/components/QueryMacro.test.tsx",
  "src/components/QueryResultContextMenu.test.tsx",
  "src/components/QueryRun11.slash.test.tsx",
  "src/components/QueryTextPane.test.tsx",
  "src/components/QueryWorkspace.test.tsx",
  "src/components/RefBlocks.orgBody.test.tsx",
  "src/components/RenderedSelectionToolbar.test.tsx",
  "src/components/RightSidebar.r1.closeSafety.test.tsx",
  "src/components/RightSidebar.test.tsx",
  "src/components/Settings.codeDisplay.test.tsx",
  "src/components/SheetBoard.test.tsx",
  "src/components/SheetGrid.interact.test.tsx",
  "src/components/SheetTable.test.tsx",
  "src/components/landingOwnership.test.tsx",
  "src/components/pageHeadEnter.test.tsx",
  "src/components/pageHeaderToast.test.tsx",
  "src/components/refOccurrence.test.tsx",
  "src/editor/coldBlock.test.tsx",
  "src/editor/ownPropertiesUI.test.tsx",
  "src/editor/reattachProperties.test.tsx",
  "src/editor/regionUI.test.tsx",
  "src/filedrop.assetBinding.test.tsx",
  "src/followLinkKeys.test.tsx",
  "src/inpageReplace.dom.test.tsx",
  "src/keybindings.p1a1.globalCapture.test.tsx",
  "src/keystrokeScaling.guard.test.tsx",
  "src/render/PeekPopup.editInPreview.test.tsx",
  "src/render/calcCopy.test.tsx",
  "src/sheet/selection.overtype.test.tsx",
  "src/textareaPrimarySelection.test.tsx",
  "src/workspaceWindows.test.tsx",
]);

/** Test files whose textarea is a different control than the block editor. */
const OTHER_TEXTAREAS = new Map<string, string>([]);

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

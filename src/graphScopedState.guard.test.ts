import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { clearOnBindingInvalidated, graphScopedSignal, refuseStaleWrite } from "./binding";
import { bumpGraphEpoch } from "./graphSession";
import { resetStore } from "./document";
import { toasts, setToasts } from "./toasts";
import {
  blockReferencesRequest, contextMenu, datePicker, exportModal, formulaEditor, openContextMenu,
  openDatePicker, openExportModal, openFormulaEditor, openPdfExport, pdfExportPage, queryBuilderAutoOpen,
  requestBlockReferences, setQueryBuilderAutoOpen,
} from "./ui";

// og 15a (K14a): a popup, editor or menu target mounted at the app root outlives
// the page it was opened on. Runtime block ids are (page path, sibling position),
// so the same id exists in every graph and a target that survived a graph switch
// writes into the new graph.
const RULE = "I-20: module state naming graph content (block id, page name, selection) must be a graphScopedSignal "
  + "(src/binding.ts), which closes it on a graph switch, and its writer must refuse a stale target. "
  + "Exemplar: formulaEditor in src/ui.ts and FormulaEditor.tsx `save`. Otherwise classify it below with the reason it cannot land in another graph";

// Module-level signals whose declared type can name graph content, and why each
// cannot write into another graph. Shrink-only: fix an entry rather than add one.
const CLASSIFIED: Record<string, string> = {
  "src/components/Block.tsx#dragId": "drag state; a click ends the drag in the old graph first, and beginDrag commits only while stillBound (asyncOwnership guard)",
  "src/components/Block.tsx#dropInd": "drop indicator of the same drag",
  "src/components/SidebarFavorites.tsx#dropTarget": "favorites drag state; a click ends the drag in the old graph first (K17b, no user path)",
  "src/components/TabBar.tsx#currentTabDropTarget": "tab drop target (pane/tab ids, not graph content)",
  "src/document/edits/selection.ts#selAnchor": "cleared by clearOnBindingInvalidated",
  "src/document/edits/selection.ts#selFocus": "cleared by clearOnBindingInvalidated",
  "src/editorController.ts#editingId": "resetStore ends the edit (endEdit \"graph-switch\")",
  "src/editorController.ts#editingOwner": "resetStore ends the edit",
  "src/editorController.ts#editingSurface": "resetStore ends the edit",
  "src/editorController.ts#caretTarget": "caret intent is consumed by the mounting editor, which resetStore ends",
  "src/editorController.ts#activeSurface": "surface key, not graph content",
  "src/pageIndex.ts#held": "read cache keyed by graph generation; resetPageIndex on switch; never written back",
  "src/pageIconBatch.ts#iconMap": "display cache, dropped on a new graph; never written back",
  "src/paneSelect.ts#paneSel": "pane layout target (pane ids, seams), not graph content",
  "src/plugins/registry.ts#registryPersistenceError": "device-local registry message",
  "src/ui.ts#accentColor": "device preference",
  "src/ui.ts#pagePropsPanel": "carries its own captureBinding(); loadGraphPath closes it and writeOne refuses when !stillBound (asyncOwnership guard)",
  "src/ui.ts#recentPages": "clearRecent() on a switch; navigation only",
  "src/ui.ts#rightSidebar": "setRightSidebar([]) on a switch; pruneSidebarBlocks on reopen",
  "src/ui.ts#lightbox": "asset URL for display only",
  "src/ui.ts#audioPlayer": "setAudioPlayer(null) on a switch; playback only",
  "src/ui.ts#switcherPluginBlock": "OwnedPluginBlockSnapshot carries its plugin graph owner",
  "src/assetCache.ts#versions": "display cache-buster per asset path; never written back",
  "src/document/save/engine.ts#conflictReasons": "resetSaveState() clears it in resetStore",
  "src/mediaEditorSettings.ts#commands": "device preference",
  "src/ui.ts#shortcutOverrides": "device preference",
  "src/ui.ts#pdfTarget": "PDF ownership: loadGraphPath retires and closes the PDF on a switch",
};

// A declared type that can hold a block id, page name, uuid or a target object.
const GRAPH_CONTENT_TYPE = /\b(?:id|ids|blockId|ownerId|bid|uuid|page|name|scope)\b|Target|Snapshot|Item|Held|string\s*\|\s*null|Record<string/;
const DECLARATION = /^(?:export )?const \[(\w+),\s*\w+\]\s*=\s*(createSignal|createStore)\s*<([\s\S]*?)>\s*\(/gm;

function productionSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return productionSources(file);
    return /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) && !file.endsWith(".d.ts") ? [file] : [];
  });
}

function graphContentSignals(): string[] {
  return productionSources("src").flatMap((file) =>
    [...readFileSync(file, "utf8").matchAll(DECLARATION)]
      .filter((match) => GRAPH_CONTENT_TYPE.test(match[3]))
      .map((match) => `${file.split(path.sep).join("/")}#${match[1]}`));
}

describe("graph-scoped UI state (I-20)", () => {
  it("every module-level signal that can name graph content is graph-scoped or classified", () => {
    const found = graphContentSignals();
    for (const key of found) expect(CLASSIFIED[key], `${RULE}: ${key}`).toBeTruthy();
    for (const key of Object.keys(CLASSIFIED)) expect(found, `stale CLASSIFIED entry ${key}; remove it`).toContain(key);
  });

  it("app-root popup writers check their target before writing", () => {
    const formula = readFileSync("src/components/FormulaEditor.tsx", "utf8");
    expect(formula, `${RULE}: FormulaEditor save`).toMatch(/const save = \(\) => \{\s*\/\/[^\n]*\n\s*if \(formulaEditor\(\) !== props\.target\) return refuseStaleWrite\(/);
    const picker = readFileSync("src/components/DatePicker.tsx", "utf8");
    expect(picker, `${RULE}: DatePicker writes`).toMatch(/const bound = \(\) => datePicker\(\) !== null \|\| \(refuseStaleWrite\(/);
    expect(picker.match(/if \(!bound\(\)\) return/g)?.length, `${RULE}: both DatePicker write paths check bound()`).toBe(2);
  });

  it("a graph-scoped signal closes on a store reset and reads null once its binding is stale", () => {
    const [value, setValue] = graphScopedSignal<{ id: string }>();
    setValue({ id: "a" });
    expect(value()).toEqual({ id: "a" });
    bumpGraphEpoch();
    expect(value(), "stale after an epoch bump").toBeNull();
    setValue({ id: "b" });
    let cleared = false;
    clearOnBindingInvalidated(() => { cleared = true; });
    resetStore();
    expect(value(), "cleared by resetStore").toBeNull();
    expect(cleared).toBe(true);
  });

  it("every app-root popup target closes on a graph switch", () => {
    openFormulaEditor({ mode: "add", ownerId: "b1", x: 0, y: 0, expr: "", formulas: [], fields: [] });
    openDatePicker("b1", "scheduled", 0, 0);
    openContextMenu(0, 0, "b1");
    openExportModal(["b1"]);
    requestBlockReferences("b1");
    setQueryBuilderAutoOpen("b1");
    openPdfExport("Page");
    const open = () => [formulaEditor(), datePicker(), contextMenu(), exportModal(), blockReferencesRequest(), queryBuilderAutoOpen(), pdfExportPage()];
    expect(open().every((target) => target !== null)).toBe(true);
    resetStore();
    expect(open()).toEqual([null, null, null, null, null, null, null]);
  });

  it("a refused stale write is visible", () => {
    setToasts([]);
    refuseStaleWrite("The formula");
    expect(toasts().map((toast) => [toast.kind, toast.message])).toEqual([
      ["error", "The formula was not saved: it was opened in a graph that is no longer open."],
    ]);
  });
});

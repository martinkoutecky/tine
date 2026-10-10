import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it, vi } from "vitest";
import { clearOnBindingInvalidated, graphScopedSignal, refuseStaleWrite } from "./binding";
import { bumpGraphEpoch } from "./graphSession";
import { resetStore } from "./document";
import { toasts, setToasts } from "./toasts";
import {
  blockReferencesRequest, contextMenu, datePicker, exportModal, formulaEditor, openContextMenu,
  openDatePicker, openExportModal, openFormulaEditor, openPdfExport, pdfExportPage, queryBuilderAutoOpen,
  requestBlockReferences, setQueryBuilderAutoOpen, journalConflicts, setJournalConflicts, syncConflicts, setSyncConflicts,
} from "./ui";
import { renderedBlocks } from "./lazyObserve";
import type { SyncConflict } from "./types";

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
  "src/components/blockGestures.ts#dragId": "drag state; a click ends the drag in the old graph first, and beginDrag commits only while bindingCurrent (asyncOwnership guard)",
  "src/components/blockGestures.ts#dropInd": "drop indicator of the same drag",
  "src/components/RightSidebar.tsx#rsDropTarget": "right-sidebar drag state (a row index and side, not graph content); pointerup ends the drag in the old graph first",
  "src/components/SidebarFavorites.tsx#dropTarget": "favorites drag state; a click ends the drag in the old graph first (K17b, no user path)",
  "src/components/DeepLinkGraphChoice.tsx#choice": "a pending choice among known graph roots for an external link (paths, not graph content); the chosen target is re-resolved through the graph-switch door, which refuses a missing page or block",
  "src/components/TabBar.tsx#currentTabDropTarget": "tab drop target (pane/tab ids, not graph content)",
  "src/draftStore.ts#held": "window-lifetime by design (storage.qnt MX): drafts of the graph that was LEFT, each carrying its own root; "
    + "the recovery panel only shows and copies them, Dismiss releases them, and nothing writes them into any graph",
  "src/document/model.ts#collapseEpochState": "a counter compared for equality by embed folds, never written back to a graph; cleared by resetStore (clearCollapseEpochs)",
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
  "src/panes.ts#layouts": "per-window pane layout trees keyed by window id (pane ids, split ratios), not graph content; page routes live in the routers",
  "src/panes.ts#focusedPanes": "focused pane id per window id, not graph content",
  "src/plugins/registry.ts#registryPersistenceError": "device-local registry message",
  "src/ui.ts#accentColor": "device preference",
  "src/ui.ts#pagePropsPanel": "carries its own captureBinding(); loadGraphPath closes it and writeOne refuses when !bindingCurrent (asyncOwnership guard)",
  "src/ui.ts#recentPages": "clearRecent() on a switch; navigation only",
  "src/ui.ts#rightSidebar": "setRightSidebar([]) on a switch; pruneSidebarBlocks on reopen",
  "src/ui.ts#lightbox": "asset URL for display only",
  "src/ui.ts#audioPlayer": "setAudioPlayer(null) on a switch; playback only",
  "src/ui.ts#switcherPluginBlock": "OwnedPluginBlockSnapshot carries its plugin graph owner",
  "src/assetCache.ts#versions": "display cache-buster per asset path; clearAssetBlobCache drops every version at graph switch",
  "src/document/save/engine.ts#conflictReasons": "resetSaveState() clears it in resetStore",
  "src/mediaEditorSettings.ts#commands": "device preference",
  "src/ui.ts#shortcutOverrides": "device preference",
  // Found once named types were expanded (og 15a follow-up):
  "src/document/model.ts#doc": "the document store itself; resetStore replaces it on every switch",
  "src/editorCommandBridge.ts#focusedEditorBridge": "the focused editor's command bridge; unregistered when that editor unmounts, and resetStore ends the edit",
  "src/editorController.ts#pendingHistoryEditorRestore": "caret selection hint consumed by the mounting editor; never written back",
  "src/favorites.ts#layout": "graph preference reseeded on open; its writes capture bindingOwner (writeGraphSignal)",
  "src/graphSession.ts#graphMeta": "the open graph's identity (the binding itself)",
  "src/plugins/manager.ts#installedPlugins": "device-level plugin catalog, not graph content",
  "src/plugins/registry.ts#communityPlugins": "remote registry catalog, not graph content",
  "src/plugins/registry.ts#communityThemes": "remote registry catalog, not graph content",
  "src/themes/manager.ts#installedThemes": "device-level theme catalog, not graph content",
  "src/toasts.ts#toasts": "notifications; their actions open settings, retry the session save, or undo (history is cleared by resetStore)",
  "src/ui.ts#journalConflicts": "cleared by clearOnBindingInvalidated; its reconcile writes capture bindingOwner at the click",
  "src/conflictQueue.ts#conflictInventory": "cleared by clearOnBindingInvalidated; its merge/discard/resolve writes capture bindingOwner at the click",
  "src/ui.ts#switcherMode": "Ctrl-K mode enum (matches only through the literal \"current-page\")",
  "src/workspaces.ts#workspaceList": "clearWorkspaces() on a switch; every workspace write captures bindingOwner",
};

// A type that can hold a block id, page name, uuid or a target object.
const GRAPH_CONTENT_TYPE = /\b(?:id|ids|blockId|ownerId|bid|uuid|page|name|scope)\b|Target|Snapshot|Item|Held|string\s*\|\s*null|Record<string/;
// Module-level signal/store: explicit type argument (group 3) or an inferred
// one, judged by its initializer (group 4).
const DECLARATION = /^(?:export )?const \[(\w+),\s*\w+\]\s*=\s*(createSignal|createStore)\s*(?:<([\s\S]*?)>\s*)?\(([^\n]*)/gm;
// A named type: an alias runs to the next top-level declaration, an interface to its closing brace.
const TYPE_ALIAS = /^(?:export )?type (\w+)\b[^=\n]*=([\s\S]*?)(?=^(?:export |const |let |function |type |interface |import |class |\/\/|\/\*)|(?![\s\S]))/gm;
const INTERFACE = /^(?:export )?interface (\w+)\b[^{\n]*\{([\s\S]*?)^\}/gm;
// A graph-scoped (popup) target, whatever its type.
const SCOPED_DECLARATION = /^(?:export )?const \[(\w+),\s*\w+\]\s*=\s*graphScopedSignal\b/gm;

type Sources = ReadonlyMap<string, string>;

function productionSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return productionSources(file);
    return /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) && !file.endsWith(".d.ts") ? [file] : [];
  });
}

function repoSources(): Map<string, string> {
  return new Map(productionSources("src").map((file) => [file.split(path.sep).join("/"), readFileSync(file, "utf8")]));
}

// Named types across all sources, so an alias (`type Pop = { blockId: string }`)
// is judged by what it holds rather than by its name.
function namedTypes(sources: Sources): Map<string, string> {
  const types = new Map<string, string>();
  for (const text of sources.values()) {
    for (const pattern of [TYPE_ALIAS, INTERFACE]) {
      for (const match of text.matchAll(pattern)) types.set(match[1], `${types.get(match[1]) ?? ""} ${match[2]}`);
    }
  }
  return types;
}

function expandTypes(text: string, types: Map<string, string>): string {
  let expanded = text;
  const seen = new Set<string>();
  for (let depth = 0; depth < 4; depth++) {
    const names = [...expanded.matchAll(/\b[A-Z]\w*\b/g)].map((m) => m[0]).filter((name) => types.has(name) && !seen.has(name));
    if (!names.length) break;
    for (const name of names) { seen.add(name); expanded += ` ${types.get(name)}`; }
  }
  return expanded;
}

export function graphContentSignals(sources: Sources = repoSources()): string[] {
  const types = namedTypes(sources);
  return [...sources].flatMap(([file, text]) =>
    [...text.matchAll(DECLARATION)]
      .filter((match) => GRAPH_CONTENT_TYPE.test(expandTypes(match[3] ?? match[4], types)))
      .map((match) => `${file}#${match[1]}`));
}

// I-21: new module-level collections must either have a binding clear or an
// explicit finite bound. Existing declarations are frozen as migration debt,
// NOT cleared by this baseline: several belong to other lane write sets.
// Exemplar: renderedBlocks in src/lazyObserve.ts, cleared by src/binding.ts.
const COLLECTION_RULE = "I-21: module Map/Set state must end with its graph or have an explicit bound; "
  + "exemplar src/lazyObserve.ts renderedBlocks and src/binding.ts clearOnBindingInvalidated. "
  + "The existing collection baseline may only shrink";
const BOUNDED_COLLECTIONS: Record<string, string> = {
  "src/assetCache.ts#cache": "MAX_CACHE_ENTRIES=128 / MAX_CACHE_BYTES=128MiB retained LRU, plus pending mounted reads",
  "src/assetCache.ts#liveEntries": "active asset leases only; last release deletes the evicted entry; clearAssetBlobCache ends graph reuse",
  "src/render/parse.ts#cache": "CACHE_MAX=8000 LRU",
  "src/workspaceWindows.ts#live": "one entry per open workspace window, at most MAX_WORKSPACE_WINDOWS (openWorkspaceWindow refuses past it); disposal deletes it, and a graph switch closes them all",
  "src/windowRealm.ts#registry": "one entry per open Tine window: main + at most MAX_WORKSPACE_WINDOWS popups; unregister on close",
  "src/windowRealm.ts#pendingFrames": "one entry per open Tine window with a frame pending; deleted when it fires or its window unregisters",
  "src/windowRealm.ts#hooks": "one entry per live onEachWindow installer (app-lifetime installers + the mounted editor); each remover deletes it",
  "src/lazyObserve.ts#nearObservers": "one IntersectionObserver per open Tine window; deleted when that window unregisters",
  "src/render/facets.ts#derived": "DERIVED_MAX=1024 LRU",
  "src/sheet/formulaEval.ts#parseCache": "clears above 500, at most 501 expressions",
  "src/components/FormulaEditor.tsx#CONDITION_OPS": "fixed COMPARISON_OPS + BOOLEAN_OPS vocabulary (8 entries), no additions",
  "src/components/FormulaEditor.tsx#VALUE_OPS": "fixed ARITHMETIC_OPS vocabulary (5 entries), no additions",
  "src/components/FormulaEditor.tsx#TRANSFORM_BY_NAME": "fixed TRANSFORMS vocabulary, no additions",
  "src/editor/autocomplete.ts#BARE_ORDER": "fixed literal command-label array, one tuple per label, no additions",
  "src/editor/autopair.ts#CLOSERS": "values of the fixed PAIRS literal, no additions",
  "src/markers.ts#OPEN_MARKERS": "subset of the 11 literal MARKERS, no additions",
  "src/sheet/aggregate.ts#AGGREGATE_SET": "fixed 15-entry AGGREGATE_FNS vocabulary, no additions",
};

interface Collection { key: string; scoped: boolean; fixed: boolean }
export function moduleCollections(sources: Sources = repoSources()): Collection[] {
  const result: Collection[] = [];
  for (const [file, text] of sources) {
    const tree = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true);
    const cleared = new Set<string>();
    const findClears = (node: ts.Node) => {
      if (ts.isCallExpression(node) && node.expression.getText(tree) === "clearOnBindingInvalidated") {
        const visit = (child: ts.Node) => {
          if (ts.isCallExpression(child) && ts.isPropertyAccessExpression(child.expression)
              && child.expression.name.text === "clear" && ts.isIdentifier(child.expression.expression)) {
            cleared.add(child.expression.expression.text);
          }
          ts.forEachChild(child, visit);
        };
        node.arguments.forEach(visit);
      }
      ts.forEachChild(node, findClears);
    };
    tree.statements.forEach((statement) => { if (ts.isExpressionStatement(statement)) findClears(statement); });
    for (const statement of tree.statements) {
      if (!ts.isVariableStatement(statement)) continue;
      for (const declaration of statement.declarationList.declarations) {
        if (!declaration.initializer) continue;
        const name = declaration.name.getText(tree);
        const allocations: ts.NewExpression[] = [];
        const visit = (node: ts.Node) => {
          // Function-local and instance collections have an ordinary owner.
          if (ts.isFunctionLike(node) || ts.isClassExpression(node)) return;
          if (ts.isNewExpression(node) && ["Map", "Set"].includes(node.expression.getText(tree))) allocations.push(node);
          ts.forEachChild(node, visit);
        };
        visit(declaration.initializer);
        for (const allocation of allocations) {
          const seed = allocation.arguments?.[0];
          const seeded = !!seed && ts.isArrayLiteralExpression(seed)
            && !seed.elements.some(ts.isSpreadElement);
          // A seeded finite vocabulary with no additions is bounded by its
          // initial entries. Include imported consumers when looking for writes.
          const mutable = [...sources.values()].some((source) => new RegExp(`\\b${name}\\s*\\.\\s*(?:add|set)\\s*\\(`).test(source));
          result.push({ key: `${file}#${name}`, scoped: cleared.has(name), fixed: seeded && !mutable });
        }
      }
    }
  }
  return result;
}

const LEGACY_COLLECTIONS = new Set([
  "src/assetRefresh.ts#pending",
  "src/binding.ts#scopedClears",
  "src/components/Macro.tsx#youtubePlayers",
  "src/conflictQueue.ts#arrivalNotices",
  "src/document/save/engine.ts#kindLedger",
  "src/document/save/engine.ts#titleIdentityIntents",
  "src/document/save/engine.ts#pageInstanceGenerations",
  "src/document/save/engine.ts#dirty",
  "src/document/save/engine.ts#baseRev",
  "src/document/save/engine.ts#deletedPages",
  "src/document/save/engine.ts#landedAliasDrafts",
  "src/document/save/engine.ts#saveChain",
  "src/document/save/engine.ts#lastSaveFailure",
  "src/document/save/engine.ts#saveFailureToasts",
  // Moved unchanged out of the save engine (step 3b P2a), shared with settle.
  "src/document/assetWrites.ts#pending",
  "src/document/save/engine.ts#groupOf",
  "src/document/save/engine.ts#sealedGroups",
  "src/document/save/engine.ts#saveAttempts",
  "src/document/save/engine.ts#deletingGroupMembers",
  "src/document/workingSet.ts#draftPins",
  // The controller contract reserves its private token even in string inventories.
  "src/editorController.ts#pending" + "FocusSurface",
  "src/editorController.ts#historyEditorTargets",
  "src/graphPreferences.ts#readers",
  "src/guide.ts#guideTitles",
  "src/guide.ts#announcementShownForRoot",
  "src/inpageFind.ts#renderedTextCache",
  "src/mediaEditorSettings.ts#commandKeys",
  "src/mediaEditorSettings.ts#commandReaders",
  "src/mock.ts#mockPluginEntries",
  "src/mock.ts#mockDrafts",
  "src/modeHooks.ts#outlineSelectionListeners",
  "src/modeHooks.ts#editingStartListeners",
  "src/modeHooks.ts#modeResetListeners",
  "src/pageIconBatch.ts#requested",
  "src/panes.ts#routers",
  "src/pdfNavigation.ts#intents",
  "src/pdfOwnership.ts#participants",
  "src/pdfOwnership.ts#mutations",
  "src/plugins/registry.ts#safetyReportRequests",
  "src/queryResultCache.ts#inFlight",
  "src/queryResultCache.ts#resolved",
  "src/referenceSectionState.ts#sections",
  "src/referenceSectionState.ts#groups",
  "src/reloadOnFocus.ts#waiters",
  "src/reloadOnFocus.ts#applications",
  "src/render/facets.ts#seeded",
  "src/resolveBatch.ts#cache",
  "src/resolveBatch.ts#resolvedCache",
  "src/resolveBatch.ts#pending",
  "src/resourceRead.ts#recorded",
  "src/sheet/matrix.ts#dimensionEntries",
  "src/sheet/queryHydration.ts#runningHydrationItems",
  "src/sheet/queryHydration.ts#pageHydrations",
  "src/sheet/queryHydration.ts#hydrationClaims",
  "src/sheet/selection.ts#lastByGrid",
  "src/sheet/selection.ts#adapters",
  "src/sheet/selection.ts#visibilityHooks",
  "src/slowBackend.ts#inFlight",
  "src/transientLayers.ts#layers",
  "src/themes/manager.ts#[revokedThemeVersions, setRevokedThemeVersions]",
]);
const LEGACY_COLLECTION_COUNT = 61;
function unownedCollections(sources: Sources): string[] {
  return moduleCollections(sources).filter((c) => !c.scoped && !c.fixed && !BOUNDED_COLLECTIONS[c.key]).map((c) => c.key);
}

// Every file that reads a graph-scoped target, whatever that target's type.
export function popupTargetConsumers(sources: Sources = repoSources()): { key: string; proven: boolean }[] {
  const getters = [...sources.values()].flatMap((text) => [...text.matchAll(SCOPED_DECLARATION)].map((match) => match[1]));
  return [...sources].flatMap(([file, text]) => getters.flatMap((getter) => {
    if (!new RegExp(`\\b${getter}\\(\\)`).test(text)) return [];
    // Proof: the read that guards a write is tied to the visible refusal
    // (`if (formulaEditor() !== props.target) return refuseStaleWrite(...)`).
    const proven = new RegExp(`\\b${getter}\\(\\)[^;\\n]*refuseStaleWrite\\(`).test(text);
    return [{ key: `${file}#${getter}`, proven }];
  }));
}

export function unprovenPopupConsumers(sources: Sources = repoSources()): string[] {
  return popupTargetConsumers(sources).filter((c) => !c.proven && !CLASSIFIED_CONSUMERS[c.key]).map((c) => c.key);
}

// Readers of a graph-scoped target that do not prove a binding check, and why
// each cannot write into another graph. Shrink-only.
const CLASSIFIED_CONSUMERS: Record<string, string> = {
  "src/components/ContextMenu.tsx#contextMenu": "menu actions run synchronously on a click while the menu is mounted, and it is mounted only "
    + "while its target is bound (reads null after a switch). Every action that awaits before writing captures bindingOwner() first "
    + "and applies nothing when stale (make template, rename, delete, open file, copy as Markdown)",
  "src/components/Page.tsx#contextMenu": "reads whether its own page-actions menu is open (aria state); no write",
  "src/components/Block.tsx#blockReferencesRequest": "opens this block's own references panel when the request names it; no write",
  "src/components/ExportModal.tsx#exportModal": "exports the ids to the clipboard; never writes the graph",
  "src/components/PdfExportDialog.tsx#pdfExportPage": "prints the named page through the OS dialog; never writes the graph",
  "src/components/QueryBuilder.tsx#queryBuilderAutoOpen": "one-shot flag compared with the mounting block's own id and consumed; no graph write",
  "src/capture.tsx#datePicker": "sizes the capture window while a picker is open; no write",
  "src/conflictPolicy.ts#heldChanges": "held external changes (always ask); Reload from disk re-dispatches the change through applyGraphChange, which drops a change of another binding generation and only re-reads disk; no graph write",
};

describe("graph-scoped UI state (I-20)", () => {
  it("late route, calendar, session and reference continuations retain their owners", () => {
    const route = readFileSync("src/router.ts", "utf8");
    expect(route).toMatch(/function openPageAtBlock[\s\S]*?const current = \(\) => stillBound\(binding\)[\s\S]*?routeIntentRevision\(\) === intent/);
    expect(route).toMatch(/const scroller = mainScroller\(\);[\s\S]*?scroller\?\.querySelector/);
    const calendar = readFileSync("src/components/CalendarJump.tsx", "utf8");
    expect(calendar).toMatch(/rev: dataRev\(\), epoch: graphEpoch\(\)/);
    expect(calendar).toMatch(/readOwned\(owner, backend\(\)\.journalContentDays\(\)\)/);
    const session = readFileSync("src/session.ts", "utf8");
    expect(session).toMatch(/const mayApply = \(\) => owner\(\) && JSON\.stringify\(buildPersistedSession\(\)\) === initialSession/);
    const block = readFileSync("src/components/Block.tsx", "utf8");
    expect(block).toMatch(/persistBlockRefTarget\([\s\S]*?\(\) => \{\s*if \(!bindingCurrent\(binding\) \|\| ac\(\) !== trigger/);
  });

  it("every module-level signal that can name graph content is graph-scoped or classified", () => {
    const found = graphContentSignals();
    for (const key of found) expect(CLASSIFIED[key], `${RULE}: ${key}`).toBeTruthy();
    for (const key of Object.keys(CLASSIFIED)) expect(found, `stale CLASSIFIED entry ${key}; remove it`).toContain(key);
  });

  it("every reader of an app-root popup target proves a binding check or is classified", () => {
    const consumers = popupTargetConsumers();
    expect(unprovenPopupConsumers(), `${RULE}. Classify a reader in CLASSIFIED_CONSUMERS only if it cannot write`).toEqual([]);
    const keys = consumers.map((c) => c.key);
    for (const key of Object.keys(CLASSIFIED_CONSUMERS)) expect(keys, `stale CLASSIFIED_CONSUMERS entry ${key}; remove it`).toContain(key);
    expect(consumers.filter((c) => c.proven).map((c) => c.key).sort(), "the proven writers").toEqual([
      "src/components/DatePicker.tsx#datePicker",
      "src/components/FormulaEditor.tsx#formulaEditor",
      "src/components/GraphNamePrompt.tsx#graphNameRequest",
      "src/components/Settings.tsx#orphanScan",
      "src/draftStore.ts#earlier",
    ]);
    const picker = readFileSync("src/components/DatePicker.tsx", "utf8");
    expect(picker.match(/if \(!bound\(\)\) return/g)?.length, `${RULE}: both DatePicker write paths check bound()`).toBe(2);
  });

  it("the guard catches a planted aliased-type popup and a planted unbound writer in an unlisted file", () => {
    const planted = new Map(repoSources());
    planted.set("src/components/PlantedPopup.tsx", [
      "type Pop = { blockId: string };",
      "export const [pop, setPop] = createSignal<Pop | null>(null);",
      "export const [inferred, setInferred] = createStore({ blockId: \"\" });",
    ].join("\n"));
    expect(graphContentSignals(planted)).toEqual(expect.arrayContaining([
      "src/components/PlantedPopup.tsx#pop", "src/components/PlantedPopup.tsx#inferred",
    ]));
    planted.set("src/plantedUi.ts", "type NoteTarget = { key: string };\nexport const [notePopup, setNotePopup] = graphScopedSignal<NoteTarget>();");
    planted.set("src/components/NotePopup.tsx", "const save = () => { const t = notePopup(); if (t) setBlockProperty(t.key, \"k\", \"v\"); };");
    expect(unprovenPopupConsumers(planted)).toEqual(["src/components/NotePopup.tsx#notePopup"]);
    planted.set("src/components/NotePopup.tsx", "const save = () => {\n  if (notePopup() !== props.target) return refuseStaleWrite(\"The note\");\n  setBlockProperty(props.target.key, \"k\", \"v\");\n};");
    expect(unprovenPopupConsumers(planted)).toEqual([]);
  });

  it("ratchets module-level Map/Set lifetimes after OG-B-LIFE", () => {
    const sources = repoSources();
    const unowned = unownedCollections(sources);
    expect(unowned.filter((key) => !LEGACY_COLLECTIONS.has(key)), COLLECTION_RULE).toEqual([]);
    expect(unowned.length, COLLECTION_RULE).toBeLessThanOrEqual(LEGACY_COLLECTION_COUNT);
    for (const key of LEGACY_COLLECTIONS) expect(unowned, `remove retired baseline entry ${key}`).toContain(key);
    const inventory = moduleCollections(sources).map((c) => c.key);
    for (const key of Object.keys(BOUNDED_COLLECTIONS)) expect(inventory).toContain(key);
  });

  it("catches new collections even inside a top-level object; accepts binding cleanup and fixed vocabularies", () => {
    const planted = new Map<string, string>([["src/planted.ts", "const state = { retained: new Map<string, string>() };\nconst seen = new Set<string>();"]]);
    expect(unownedCollections(planted)).toEqual(["src/planted.ts#state", "src/planted.ts#seen"]);
    planted.set("src/planted.ts", "const seen = new Set<string>();\nclearOnBindingInvalidated(() => { seen.clear(); });\nconst vocabulary = new Set(['a', 'b']);");
    expect(unownedCollections(planted)).toEqual([]);
    planted.set("src/consumer.ts", "vocabulary.add(userInput);");
    expect(unownedCollections(planted)).toEqual(["src/planted.ts#vocabulary"]);
    planted.set("src/planted.ts", "const loaded = new Set(loadGraph()); const spread = new Set([...loadGraph()]);");
    expect(unownedCollections(planted)).toEqual(["src/planted.ts#loaded", "src/planted.ts#spread"]);
  });

  it("L15:81: rendered block markers end with the graph", () => {
    renderedBlocks.add("old-graph-block");
    resetStore();
    expect(renderedBlocks.size, "I-21: no retired graph render markers").toBe(0);
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

  it("one throwing clear neither aborts the other clears nor the rest of resetStore, and is surfaced (I-9)", () => {
    setToasts([]);
    const [value, setValue] = graphScopedSignal<{ id: string }>();
    let armed = true;
    clearOnBindingInvalidated(() => { if (armed) { armed = false; throw new Error("planted clear failure"); } });
    let later = false;
    clearOnBindingInvalidated(() => { later = true; });
    setValue({ id: "a" });
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      expect(() => resetStore(), "resetStore must finish despite a throwing clear").not.toThrow();
    } finally {
      errors.mockRestore();
    }
    expect(later, "a clear registered after the throwing one still runs").toBe(true);
    expect(value(), "graph-scoped signals are still cleared").toBeNull();
    expect(toasts().some((toast) => toast.kind === "error" && toast.message.includes("binding.clear")),
      "the failure is visible with its fixed family").toBe(true);
  });

  it("the reconcile lists (duplicate journal days, sync conflicts) close on a graph switch", () => {
    // Their Merge/Rename/Trash actions send graph-relative paths to whichever
    // graph is open at the click; a list surviving a switch (or a failed
    // refresh) would act on the same-named file of the new graph.
    setJournalConflicts([{ title: "Jun 26th, 2026", files: [{ name: "a.md", path: "journals/a.md", preview: "", canonical: false }] }]);
    setSyncConflicts([{ path: "pages/a.sync-conflict-1.md", base_name: "a", base_path: "pages/a.md", kind: "page" } as SyncConflict]);
    resetStore();
    expect(journalConflicts()).toEqual([]);
    expect(syncConflicts()).toEqual([]);
  });

  it("a refused stale write is visible", () => {
    setToasts([]);
    refuseStaleWrite("The formula");
    expect(toasts().map((toast) => [toast.kind, toast.message])).toEqual([
      ["error", "The formula was not saved: it was opened in a graph that is no longer open."],
    ]);
  });
});

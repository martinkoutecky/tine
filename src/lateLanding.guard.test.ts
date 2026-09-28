import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

function productionSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? productionSources(file) : /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) ? [file] : [];
  });
}

// Named rule exemptions are pinned to exact backend method counts below.
const EXEMPT: Record<string, string> = {
  "src/components/Block.tsx#Editor.capturePhotoCmd": "asset editor token checks binding before insertion and every error toast",
  "src/components/Block.tsx#Editor.voiceMemoToggle": "native recorder must be cancelled from a stale start result; editor token guards insertion and toasts",
  "src/debug.ts#initDebug": "one-time device debug probe has no graph or route landing",
  "src/graph.ts#loadGraphPath": "the graph transition changes its own binding; its transition lock owns publication",
  "src/plugins/manager.ts#uninstall": "device-local plugin removal completes in the process-wide manager across graph navigation",
  "src/plugins/registry.ts#loadVerifiedCachedRegistry": "one-time verified cache migration uses native expected-legacy comparison before publication",
  "src/assetSettings.ts#initAssetSettings": "preference revision gates the device-local signal",
  "src/backgroundFlush.ts#installBackgroundFlush.flush": "close coordinator owns and reports flush outcome",
  "src/blockRefCounts.ts#<module>": "graph epoch and cached query own publication",
  "src/clipboard.ts#copyBlockOutline": "clipboard slot generation owns rollback of a failed native write",
  "src/components/AboutTab.tsx#openExternal": "external link has no in-app result landing",
  "src/components/HelpShortcuts.tsx#openExternal": "external link has no in-app result landing",
  "src/components/Settings.tsx#ConflictFileRow": "captured file row checks graph binding before landing",
  "src/copySettings.ts#initCopySettings": "preference revision gates each device-local signal",
  "src/debug.ts#dbg": "best-effort process log has no UI landing",
  "src/editor/linkDefault.ts#initLinkDefault": "migration write reports failure independently of the current policy request",
  "src/editor/linkDefault.ts#setLinkFirstMatch": "legacy preference write reports failure independently",
  "src/graphSession.ts#setJournalTemplate": "graph binding owns the template update",
  "src/launcherRanking.ts#initLauncherRankingSetting": "preference revision gates the device-local signal",
  "src/localFileSettings.ts#initLocalFileSettings": "preference revision gates the device-local signal",
  "src/mediaEditorSettings.ts#initMediaEditorSettings": "preference revision gates the device-local signal",
  "src/nativeChrome.ts#setNativeFrame": "device-local write reports its own failure",
  "src/nativeChrome.ts#initNativeChrome": "preference revision gates the device-local signal",
  "src/navSettings.ts#initNavSettings": "preference revision gates the device-local signal",
  "src/plugins/manager.ts#loadSettings": "process-wide plugin settings have no graph owner",
  "src/plugins/manager.ts#storeSettings": "process-wide plugin settings queue owns writes",
  "src/plugins/registry.ts#loadSafetyReport": "process-wide safety cache owns its publication",
  "src/refCompletionSettings.ts#initRefCompletionSettings": "preference revision gates the device-local signal",
  "src/spellcheckSettings.ts#apply": "device spellcheck operation reports its own failure",
  "src/spellcheckSettings.ts#initSpellcheckSettings": "preference revision gates each device-local signal",
  "src/themeGallery.ts#initThemeGallery": "preference revision gates the device-local signal",
  "src/themes/manager.ts#persist": "process-wide theme setting write reports failure",
  "src/themes/manager.ts#initThemePackages": "process-wide theme setting read has no graph owner",
  "src/ui.ts#applyTheme": "device system-bar effect has no asynchronous UI landing",
  "src/ui.ts#toggleTheme": "device system-bar effect has no asynchronous UI landing",
  "src/update.ts#openReleases": "external link has no in-app result landing",
  "src/backend.ts#saveOnePage": "save refusal adapter preserves family and disk revision; save/reviewFixes.test.ts and save/cadenceConflict.test.ts own reconciliation",
  "src/document/save/engine.ts#createPage": "save state machine owns create refusal; save/reviewFixes.test.ts",
  "src/document/save/engine.ts#deletePageOnDisk": "save state machine owns deletion outcome; save/groupDeletion.test.ts",
  "src/document/save/engine.ts#doSave": "save state machine owns dirty and conflict reconciliation; save/cadenceConflict.test.ts",
  "src/document/save/engine.ts#resolveConflict": "save state machine owns conflict retry; save/reviewFixes.test.ts",
  "src/document/save/engine.ts#runGroup": "save state machine owns grouped publication and rollback; save/groups.test.ts",
  "src/gpu.ts#warnIfSoftwareRendering": "one-time device-wide best-effort probe has no graph or view owner",
  "src/plugins/manager.ts#install": "process-wide install writes and applies its returned record independent of graph navigation; plugins/manager.test.ts",
  "src/plugins/registry.ts#verifiedIndex": "pure process-wide signature check; callers own any UI publication; plugins/registry.test.ts",
};
// Exact backend methods and occurrence counts inside each exempt function.
// A new call to the same method or any new method must fail the guard.
const EXEMPT_CALLS: Record<string, string[]> = {
  "src/components/Block.tsx#Editor.capturePhotoCmd": ["capturePhoto", "importNativeCapture"],
  "src/components/Block.tsx#Editor.voiceMemoToggle": ["stopRecording", "importNativeCapture", "startRecording"],
  "src/debug.ts#initDebug": ["debugInfo"],
  "src/graph.ts#loadGraphPath": ["loadGraph"],
  "src/plugins/manager.ts#uninstall": ["uninstallPlugin", "setAppString"],
  "src/plugins/registry.ts#loadVerifiedCachedRegistry": ["loadPluginRegistryCache", "storePluginRegistryCache"],
  "src/assetSettings.ts#initAssetSettings": ["getAppString"],
  "src/backgroundFlush.ts#installBackgroundFlush.flush": ["flushAll", "flushAll"],
  "src/blockRefCounts.ts#<module>": ["getBlockRefCounts"],
  "src/clipboard.ts#copyBlockOutline": ["writeRich"],
  "src/components/AboutTab.tsx#openExternal": ["openExternal"],
  "src/components/HelpShortcuts.tsx#openExternal": ["openExternal"],
  "src/components/Settings.tsx#ConflictFileRow": ["readJournalFile"],
  "src/copySettings.ts#initCopySettings": ["getAppBool", "getAppBool", "getAppBool"],
  "src/debug.ts#dbg": ["debugLog"],
  "src/editor/linkDefault.ts#initLinkDefault": ["setAppString"],
  "src/editor/linkDefault.ts#setLinkFirstMatch": ["setLinkFirstMatch"],
  "src/graphSession.ts#setJournalTemplate": ["setDefaultJournalTemplate"],
  "src/launcherRanking.ts#initLauncherRankingSetting": ["getAppBool"],
  "src/localFileSettings.ts#initLocalFileSettings": ["getAppBool"],
  "src/mediaEditorSettings.ts#initMediaEditorSettings": ["getAppString", "getAppString"],
  "src/nativeChrome.ts#setNativeFrame": ["setAppBool"],
  "src/nativeChrome.ts#initNativeChrome": ["getAppBool"],
  "src/navSettings.ts#initNavSettings": ["getAppBool"],
  "src/plugins/manager.ts#loadSettings": ["getAppString"],
  "src/plugins/manager.ts#storeSettings": ["setAppString", "setAppString"],
  "src/plugins/registry.ts#loadSafetyReport": ["setAppString", "getAppString"],
  "src/refCompletionSettings.ts#initRefCompletionSettings": ["getAppBool"],
  "src/spellcheckSettings.ts#apply": ["applySpellcheck"],
  "src/spellcheckSettings.ts#initSpellcheckSettings": ["getAppBool", "getAppString"],
  "src/themeGallery.ts#initThemeGallery": ["getAppString"],
  "src/themes/manager.ts#persist": ["setAppString"],
  "src/themes/manager.ts#initThemePackages": ["getAppString"],
  "src/ui.ts#applyTheme": ["setSystemBarAppearance"],
  "src/ui.ts#toggleTheme": ["setSystemBarAppearance"],
  "src/update.ts#openReleases": ["openExternal"],
  "src/backend.ts#saveOnePage": ["savePages"],
  "src/document/save/engine.ts#createPage": ["resolvePage"],
  "src/document/save/engine.ts#deletePageOnDisk": ["deletePage", "deletePage"],
  "src/document/save/engine.ts#doSave": ["resolvePage", "getPageByPath"],
  "src/document/save/engine.ts#resolveConflict": ["getPageByPath", "getPage"],
  "src/document/save/engine.ts#runGroup": ["getPageByPath", "savePages"],
  "src/gpu.ts#warnIfSoftwareRendering": ["gpuEnv"],
  "src/plugins/manager.ts#install": ["installPlugin"],
  "src/plugins/registry.ts#verifiedIndex": ["verifyPluginRegistry"],
};
const OWNERS = new Set(["graphOwner", "ownedWhen", "latestOwner", "revisionOwner"]);
const BOUNDARIES = new Set(["readOwned", "readOwnedResource", "writeOwned", "serializeOwned", "serializeDurable"]);
// Durable backend operations are classified by interface verb, including names
// such as rename and paste that a write-prefix expression cannot recognize.
const DURABLE_BACKEND_METHODS = new Set([
  "approveExternalAssets", "forgetKnownGraph", "installPlugin", "uninstallPlugin", "setPluginEnabled",
  "storePluginRegistryCache", "setSystemBarAppearance", "createGraph", "savePages", "copyGuideIntoGraph",
  "setGuideAnnounced", "deletePage", "renamePage", "publishHtml", "setFavorites",
  "setPreferredWorkflow", "setTimetrackingEnabled", "setShowBrackets", "setDocModeEnterForNewBlock",
  "setLogicalOutdenting", "setPreferredFormat", "setJournalTitleFormat", "setDefaultJournalTemplate",
  "setStartOfWeek", "editAssetExternal", "trashAsset", "emptyAssetTrash", "trashJournalFile",
  "applyJournalFilenameMigrations",
  "mergePages", "renameFileToPage", "resolveSyncConflict", "trashSyncConflict", "saveAsset",
  "pasteImage", "importAsset", "importNativeCapture", "writeText", "writeRich", "copyImageToClipboard",
  "openPdf", "writeHighlights", "writePdfViewState", "savePdfAreaImage", "rollbackPdfAreaImage",
  "setBackupKeep", "setCaptureEnterFiles", "setLinkFirstMatch", "setWatchMode", "restoreBackup",
  "saveSession", "saveWorkspaces", "setSmoothScroll", "setAppBool", "setAppString", "applySpellcheck",
  "debugLog",
]);
// All remaining Backend methods are reads, resource subscriptions, dialogs,
// transient OS controls, or graph-binding controls. Adding a method requires
// an explicit durable/non-durable decision in the census test below.
const NON_DURABLE_BACKEND_METHODS = new Set([
  "graphBindingGeneration", "inspectGraphAccess", "loadGraph", "openGraphWindow", "startupGraphPath",
  "captureTarget", "bindCaptureGraph", "listKnownGraphs", "appPlatform", "listInstalledPlugins",
  "readPluginEntry", "verifyPluginRegistry", "loadPluginRegistryCache", "defaultGraphParent", "quit",
  "closeGraphWindow", "openDevtools", "pageInventory", "journalFeedPage", "journalContentDays",
  "getPage", "resolvePage", "graphSourceFiles", "guidePages", "getBacklinks",
  "getBacklinkFilterContext", "getUnlinkedRefs", "warmDone", "getBlockRefCounts", "getBlockReferrers",
  "pagePrintHtml", "runQuery", "exportQuerySubtrees", "runAdvancedQuery", "parseQuery", "printQuery", "queryOgExpressible", "queryRegistry",
  "queryRun", "queryExplainEmpty", "queryFacets", "pageIcons",
  "readCustomCss", "openExternal", "openAsset", "openPageFile", "detectMediaEditor", "listOrphanAssets",
  "assetTrashStats", "listJournalConflicts", "listJournalFilenameMigrations", "readJournalFile", "getPageByPath", "listSyncConflicts",
  "syncConflictDiff", "onConflictsChanged", "search", "runGraphSearch", "quickSwitch", "captureQuickSwitch",
  "listTemplates", "resolveBlock", "resolveBlocks", "previewBlock", "readAsset", "streamAsset",
  "readLocalImage", "readClipboardImage", "clipboardFiles", "readTextFile", "confirm", "pickFolder",
  "pickGraphFolder", "pickFile", "capturePhoto", "startRecording", "stopRecording", "cancelRecording",
  "readHighlights", "onGraphChanged", "getBackupKeep", "getCaptureEnterFiles", "getLinkFirstMatch",
  "getWatchMode", "listBackups", "loadSession", "loadWorkspaces", "gpuEnv", "getSmoothScroll",
  "getAppBool", "getAppString", "listSpellcheckDictionaries", "debugInfo",
]);
// These helpers receive Owner from audited constructor call sites. Arbitrary
// Owner parameters do not prove provenance to this syntax scan.
const OWNER_PARAM_HELPERS = new Set([
  "src/App.tsx#installMobileExternalLinkHandler",
  "src/carry.ts#ensureLoaded",
  "src/carry.ts#ensureToday",
  "src/components/ExportModal.tsx#warmMacro",
  "src/components/ExportModal.tsx#warmQueryMacros",
  "src/devtools/lsdoc-diff/orchestrator.ts#runComparison",
  "src/guide.ts#markGuideAnnounced",
]);

function functionName(node: ts.Node): string {
  const names: string[] = [];
  for (let parent = node.parent; parent; parent = parent.parent) {
    if (ts.isFunctionDeclaration(parent) && parent.name) names.push(parent.name.text);
    if (ts.isMethodDeclaration(parent) && parent.name) names.push(parent.name.getText());
    if ((ts.isArrowFunction(parent) || ts.isFunctionExpression(parent)) && parent.parent) {
      const declaration = parent.parent;
      if (ts.isVariableDeclaration(declaration) && ts.isIdentifier(declaration.name)) names.push(declaration.name.text);
      if (ts.isPropertyAssignment(declaration)) names.push(declaration.name.getText());
    }
  }
  return names.reverse().join(".") || "<module>";
}

/** Report backend completions outside the owned-result boundary. This syntax
 * guard checks owner provenance; behavior tests must check the caller's stale
 * result branch separately. */
export function lateLandingViolations(file: string, source: string): string[] {
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const ownedConstructors = new Set<string>();
  for (const statement of tree.statements) {
    if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier) ||
      !/\/?owned$/.test(statement.moduleSpecifier.text)) continue;
    const names = statement.importClause?.namedBindings;
    if (!names || !ts.isNamedImports(names)) continue;
    for (const specifier of names.elements) {
      if (OWNERS.has(specifier.propertyName?.text ?? specifier.name.text)) ownedConstructors.add(specifier.name.text);
    }
  }
  const aliases = new Set(["api", "deps"]);
  const collectAliases = (node: ts.Node): void => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer &&
      ts.isCallExpression(node.initializer) && ts.isIdentifier(node.initializer.expression) &&
      node.initializer.expression.text === "backend") aliases.add(node.name.text);
    ts.forEachChild(node, collectAliases);
  };
  collectAliases(tree);
  const backendMethod = (node: ts.Node): string | null => {
    if (!ts.isCallExpression(node) || !ts.isPropertyAccessExpression(node.expression)) return null;
    const receiver = node.expression.expression;
    const direct = ts.isCallExpression(receiver) && ts.isIdentifier(receiver.expression) && receiver.expression.text === "backend";
    if (direct || (ts.isIdentifier(receiver) && aliases.has(receiver.text))) return node.expression.name.text;
    return null;
  };
  const promiseAliases = new Map<string, ts.Expression>();
  const collectPromises = (node: ts.Node): void => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer &&
      backendMethod(node.initializer)) promiseAliases.set(node.name.text, node.initializer);
    ts.forEachChild(node, collectPromises);
  };
  collectPromises(tree);
  const backendCalls = (node: ts.Node): string[] => {
    const found: string[] = [];
    const seen = new Set<string>();
    const walk = (child: ts.Node): void => {
      if (ts.isCallExpression(child) && ts.isIdentifier(child.expression) &&
        BOUNDARIES.has(child.expression.text)) return;
      if (ts.isIdentifier(child) && promiseAliases.has(child.text) && !seen.has(child.text)) {
        seen.add(child.text);
        walk(promiseAliases.get(child.text)!);
      }
      const method = backendMethod(child);
      if (method) found.push(method);
      ts.forEachChild(child, walk);
    };
    walk(node);
    return found;
  };
  const shadowsImport = (call: ts.CallExpression, name: string): boolean => {
    for (let scope: ts.Node | undefined = call.parent; scope && !ts.isSourceFile(scope); scope = scope.parent) {
      if (ts.isFunctionLike(scope) && scope.parameters.some((param) =>
        ts.isIdentifier(param.name) && param.name.text === name)) return true;
      if (ts.isCatchClause(scope) && scope.variableDeclaration &&
        ts.isIdentifier(scope.variableDeclaration.name) && scope.variableDeclaration.name.text === name) return true;
      if (ts.isBlock(scope)) {
        for (const statement of scope.statements) {
          if (ts.isFunctionDeclaration(statement) && statement.name?.text === name) return true;
          if (ts.isVariableStatement(statement) && statement.declarationList.declarations.some((declaration) =>
            ts.isIdentifier(declaration.name) && declaration.name.text === name)) return true;
        }
      }
    }
    return false;
  };
  const ownerValid = (arg: ts.Expression | undefined): boolean => {
    if (!arg) return false;
    if (ts.isCallExpression(arg) && ts.isIdentifier(arg.expression))
      return ownedConstructors.has(arg.expression.text) && !shadowsImport(arg, arg.expression.text);
    if (ts.isPropertyAccessExpression(arg) && arg.name.text === "owner")
      return file === "src/workspaces.ts" && ts.isIdentifier(arg.expression) && arg.expression.text === "scope";
    if (ts.isIdentifier(arg)) {
      for (let parent: ts.Node | undefined = arg.parent; parent; parent = parent.parent) {
        if (ts.isBlock(parent) || ts.isSourceFile(parent)) {
          for (const statement of parent.statements) {
            if (statement.getStart(tree) >= arg.getStart(tree) || !ts.isVariableStatement(statement)) continue;
            for (const declaration of statement.declarationList.declarations) {
              if (!ts.isIdentifier(declaration.name) || declaration.name.text !== arg.text) continue;
              return ownerValid(declaration.initializer);
            }
          }
        }
        if (ts.isFunctionLike(parent) && parent.parameters.some((param) =>
          ts.isIdentifier(param.name) && param.name.text === arg.text && param.type?.getText(tree) === "Owner"))
          return OWNER_PARAM_HELPERS.has(`${file}#${functionName(arg)}`);
      }
      return false;
    }
    return false;
  };
  const guarded = (node: ts.Node): boolean => {
    const boundaryOwner = (call: ts.CallExpression) => call.arguments[
      call.expression.getText(tree).startsWith("serialize") ? 1 : 0
    ];
    if (ts.isAwaitExpression(node) && ts.isCallExpression(node.expression) &&
      ts.isIdentifier(node.expression.expression) &&
      BOUNDARIES.has(node.expression.expression.text) && ownerValid(boundaryOwner(node.expression))) return true;
    for (let parent = node.parent; parent; parent = parent.parent) {
      if (ts.isCallExpression(parent) && ts.isIdentifier(parent.expression) &&
        BOUNDARIES.has(parent.expression.text) && ownerValid(boundaryOwner(parent))) return true;
      if (ts.isStatement(parent)) break;
    }
    return false;
  };
  const violations: string[] = [];
  const visit = (node: ts.Node): void => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && BOUNDARIES.has(node.expression.text) &&
      !ownerValid(node.arguments[node.expression.text.startsWith("serialize") ? 1 : 0]) && node.arguments.slice(1).some((arg) => backendCalls(arg).length)) {
      const line = tree.getLineAndCharacterOfPosition(node.getStart(tree)).line + 1;
      violations.push(`${file}#${functionName(node)}:${line}:${node.expression.text}: owner must come from owned.ts constructor`);
    }
    const thenReceiver = ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression)
      ? node.expression.expression : null;
    const ownedReceiver = thenReceiver && ts.isCallExpression(thenReceiver) &&
      ts.isIdentifier(thenReceiver.expression) && BOUNDARIES.has(thenReceiver.expression.text) && ownerValid(thenReceiver.arguments[thenReceiver.expression.text.startsWith("serialize") ? 1 : 0]);
    const isThen = thenReceiver && !ownedReceiver && !ts.isAwaitExpression(node.parent) &&
      ts.isPropertyAccessExpression((node as ts.CallExpression).expression) &&
      ["then", "catch", "finally"].includes((node as ts.CallExpression & { expression: ts.PropertyAccessExpression }).expression.name.text) &&
      backendCalls(thenReceiver).length > 0;
    if (((ts.isAwaitExpression(node) && backendCalls(node.expression).length > 0) || isThen) && !guarded(node)) {
      const line = tree.getLineAndCharacterOfPosition(node.getStart(tree)).line + 1;
      violations.push(`${file}#${functionName(node)}:${line}:${backendCalls(ts.isAwaitExpression(node) ? node.expression : thenReceiver!).join(",")}: backend completion bypasses owned result`);
    }
    ts.forEachChild(node, visit);
  };
  visit(tree);
  return violations;
}

function durableReadViolations(file: string, source: string): string[] {
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const violations: string[] = [];
  const visit = (node: ts.Node): void => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "readOwned") {
      const work = node.arguments[1];
      const walk = (candidate: ts.Node): void => {
        if (ts.isCallExpression(candidate) && ts.isPropertyAccessExpression(candidate.expression) &&
          DURABLE_BACKEND_METHODS.has(candidate.expression.name.text) &&
          ts.isCallExpression(candidate.expression.expression) &&
          ts.isIdentifier(candidate.expression.expression.expression) &&
          candidate.expression.expression.expression.text === "backend") {
          violations.push(`${file}#${functionName(node)}: readOwned swallows stale durable ${candidate.expression.name.text} failure`);
        }
        ts.forEachChild(candidate, walk);
      };
      if (work) walk(work);
    }
    ts.forEachChild(node, visit);
  };
  visit(tree);
  return violations;
}

function assertLateLandings(file: string, source: string): void {
  const violations = lateLandingViolations(file, source);
  if (violations.length) throw new Error(
    `I-20: awaited backend calls and .then/.catch/.finally continuations need an owned.ts boundary or an exact rule exemption; exemplar src/components/Page.tsx runJournalFeedRestart.\n${violations.join("\n")}`
  );
}

describe("I-20 owned backend completion syntax", () => {
  it("keeps production backend completions behind owned results", () => {
    const violations = productionSources("src").flatMap((file) => lateLandingViolations(file, readFileSync(file, "utf8")));
    const observed = new Map<string, string[]>();
    for (const violation of violations) {
      const [key, , method] = violation.split(":");
      observed.set(key, [...(observed.get(key) ?? []), method]);
    }
    const pinned = EXEMPT_CALLS;
    expect(Object.keys(EXEMPT).sort()).toEqual(Object.keys(EXEMPT_CALLS).sort());
    expect(violations.filter((violation) => !pinned[violation.split(":")[0]]),
      "I-20: backend completions require owned results; exemplar Page.tsx runJournalFeedRestart").toEqual([]);
    for (const [key, calls] of Object.entries(pinned)) {
      expect((observed.get(key) ?? []).sort(), `I-20: ${key} permits only its pinned backend methods and counts`).toEqual([...calls].sort());
    }
  });
  it("keeps durable backend writes out of readOwned", () => {
    const backendSource = ts.createSourceFile("src/backend.ts", readFileSync("src/backend.ts", "utf8"), ts.ScriptTarget.Latest, true);
    const api = backendSource.statements.find((statement): statement is ts.InterfaceDeclaration =>
      ts.isInterfaceDeclaration(statement) && statement.name.text === "Backend");
    const methods = api?.members.filter(ts.isMethodSignature).map((method) => method.name.getText(backendSource)) ?? [];
    expect([...DURABLE_BACKEND_METHODS, ...NON_DURABLE_BACKEND_METHODS].sort(),
      "I-9: classify every Backend method as durable or non-durable; exemplar renamePage").toEqual(methods.sort());
    const violations = productionSources("src").flatMap((file) => durableReadViolations(file, readFileSync(file, "utf8")));
    expect(violations, "I-9: durable writes use writeOwned; exemplar Settings BackupsTab.saveKeep").toEqual([]);
    expect(durableReadViolations("src/planted.ts", "readOwned(graphOwner(), backend().setBackupKeep(3))")).toHaveLength(1);
    expect(durableReadViolations("src/planted.ts", "readOwned(graphOwner(), backend().renamePage('A', 'B', 'rename-page'))")).toHaveLength(1);
    expect(durableReadViolations("src/planted.ts", "readOwned(graphOwner(), backend().pasteImage(1))")).toHaveLength(1);
    expect(durableReadViolations("src/planted.ts", "readOwned(graphOwner(), backend().openPdf('a.pdf', 'A', 'create-page', 1))")).toHaveLength(1);
  });
  it("fails a planted old graph completion", () => {
    expect(() => assertLateLandings("src/planted.ts", "async function stale() { const dto = await backend().getPage('P', 'page'); reloadPage(dto); }")).toThrow(/I-20.*exemplar src\/components\/Page\.tsx/s);
  });
  it("finds multiline then, nested await and a backend alias", () => {
    expect(lateLandingViolations("src/planted.ts", "backend().getPage('a', 'page')\n .then((dto) => setPage(dto));")).toHaveLength(1);
    expect(lateLandingViolations("src/planted.ts", "function outer() { return items.map(async () => { const dto = await backend().getPage('a', 'page'); setPage(dto); }); }")).toHaveLength(1);
    expect(lateLandingViolations("src/planted.ts", "async function alias() { const api = backend(); const dto = await api.getPage('a', 'page'); setPage(dto); }")).toHaveLength(1);
    expect(lateLandingViolations("src/planted.ts", "import { graphOwner } from './owned'; async function safe() { const result = await readOwned(graphOwner(), backend().getPage('a', 'page')); if (result.kind === 'stale') return; setPage(result.value); }")).toHaveLength(0);
    expect(lateLandingViolations("src/planted.ts", "async function alias() { const p = backend().getPage('a', 'page'); const dto = await p; setPage(dto); }")).toHaveLength(1);
    expect(lateLandingViolations("src/planted.ts", "backend().getPage('a', 'page').catch((e) => setError(e));")).toHaveLength(1);
    expect(lateLandingViolations("src/planted.ts", "backend().getPage('a', 'page').finally(() => setDone(true));")).toHaveLength(1);
    expect(lateLandingViolations("src/planted.ts", "await readOwned(() => true, backend().getPage('a', 'page')); ")).not.toHaveLength(0);
    expect(lateLandingViolations("src/planted.ts", "const fake = () => true; await readOwned(fake, backend().getPage('a', 'page')); ")).not.toHaveLength(0);
    expect(lateLandingViolations("src/planted.ts", "function graphOwner() { return () => true; } await readOwned(graphOwner(), backend().getPage('a', 'page')); ")).not.toHaveLength(0);
    expect(lateLandingViolations("src/planted.ts", "import { graphOwner, readOwned } from './owned'; async function f(graphOwner: () => Owner) { const r = await readOwned(graphOwner(), backend().getPage('P', 'page')); setPage((r as any).value); }")).not.toHaveLength(0);
    expect(lateLandingViolations("src/planted.ts", "async function fake(owner: Owner) { await readOwned(owner, backend().getPage('a', 'page')); }")).not.toHaveLength(0);
  });
  it("does not exempt a new backend method inside an exempt function", () => {
    const planted = lateLandingViolations("src/graph.ts",
      "async function loadGraphPath() { await backend().loadGraph('a'); await backend().getPage('P', 'page'); }");
    const methods = planted.map((violation) => violation.split(":")[2]).sort();
    expect(methods).not.toEqual([...EXEMPT_CALLS["src/graph.ts#loadGraphPath"]].sort());
    expect(methods).toContain("getPage");
  });
});

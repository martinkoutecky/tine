import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

// Which regions must be able to fail alone.
//
// Solid throws the WHOLE pending effect queue away when a render throws
// (`runUpdates`, solid-js dist/solid.js:820 — `if (!wait) Effects = null;`) and
// then RETHROWS when no boundary is registered. Tine registered none, so one
// unreadable value blanked the entire window silently and did not recover:
// GH #490 found it in the conflict panel, GH #332 is a user who has been on an
// old release since August because of it.
//
// A boundary is therefore an architectural fact, not a decoration, and facts
// live in tests. Each entry below is a mount site whose failure must cost the
// user that region and nothing more. Adding a seam is welcome; removing one
// means arguing that a throw there should take the app with it.
const REQUIRED_SEAMS: Array<{ file: string; component: string }> = [
  { file: "src/App.tsx", component: "PaneContent" },
  { file: "src/App.tsx", component: "Sidebar" },
  ...["KeyedPdfViewer", "RightSidebar", "UnsavedRecovery", "WelcomeLayer", "CalendarJump", "WorkspaceSwitcher"].map(component => ({ file: "src/App.tsx", component })),
  // OG-MULTIWINDOW: the app overlays render in whichever window the user is in.
  ...["QuickSwitcher", "ContextMenu", "DatePicker", "FormulaEditor", "PageProps", "ExportModal", "PdfExportDialog", "QueryExportDialog", "Settings", "HelpPopup", "Lightbox", "AudioOverlay"].map(component => ({ file: "src/components/WindowOverlays.tsx", component })),
  { file: "src/components/WorkspaceWindowShell.tsx", component: "TabBar" },
  { file: "src/components/RightSidebar.tsx", component: "SidebarItemView" },
  { file: "src/components/Macro.tsx", component: "QueryMacroContent" },
  { file: "src/components/QueryLivePreview.tsx", component: "QueryLivePreviewContent" },
  { file: "src/components/PluginsTab.tsx", component: "PluginsTabContent" },

  { file: "src/components/Page.tsx", component: "LinkedReferences" },
  { file: "src/components/Page.tsx", component: "UnlinkedReferences" },
  { file: "src/components/Page.tsx", component: "PageConflictResolution" },
  { file: "src/components/RightSidebar.tsx", component: "LinkedReferences" },
  { file: "src/components/RightSidebar.tsx", component: "UnlinkedReferences" },
];

const BOUNDARY = "FailureBoundary";
const REPO_ROOT = path.resolve(__dirname, "..");

function parse(file: string, source: string): ts.SourceFile {
  return ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
}

function tagName(node: ts.Node): string | null {
  if (ts.isJsxSelfClosingElement(node)) return node.tagName.getText();
  if (ts.isJsxElement(node)) return node.openingElement.tagName.getText();
  return null;
}

/**
 * Mount sites of `component` in `source` that no enclosing element wraps in a
 * FailureBoundary. AST, not text: a comment or a doc string naming the
 * component is prose and must stay legal, and the first thing a text scan
 * catches is the comment explaining the rule.
 */
export function unboundedMountSites(file: string, source: string, component: string): number[] {
  const sourceFile = parse(file, source);
  const offending: number[] = [];
  const visit = (node: ts.Node) => {
    if (tagName(node) === component) {
      let guarded = false;
      for (let parent = node.parent; parent; parent = parent.parent) {
        if (tagName(parent) === BOUNDARY) {
          guarded = true;
          break;
        }
      }
      if (!guarded) {
        offending.push(sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1);
      }
    }
    ts.forEachChild(node, visit);
  };
  ts.forEachChild(sourceFile, visit);
  return offending;
}

/** Mount sites of `component` in `source`, wrapped or not. */
function mountCount(file: string, source: string, component: string): number {
  let count = 0;
  const visit = (node: ts.Node) => {
    if (tagName(node) === component) count++;
    ts.forEachChild(node, visit);
  };
  ts.forEachChild(parse(file, source), visit);
  return count;
}

// A window root's structural shells coordinate layout/drawers; its independently
// loaded child surfaces must own a boundary. Scan unknown mounts too, so adding a
// new dialog without adding a name to REQUIRED_SEAMS still fails (I-20).
const APP_SHELLS = new Set(["Show", "Suspense", "FailureBoundary", "DrawerBackground",
  "MobileDrawerPanel", "MobileDrawerController", "PaneTree", "PaneEdgeHighlights",
  "PaneSelectHint", "ResizeGrips", "Toasts",
  // Context providers carry a value; their children are judged as the root's own.
  "WindowContext.Provider"]);
/** Shells that are pure containers of top-level surfaces: the scan follows the
 * import and judges their mounts as if they were the root's own (review F7). */
const CONTAINERS = new Set(["WindowOverlays"]);
/** Window roots that are not workspace surfaces, with the reason. */
const ROOT_EXEMPT: Record<string, string> = {
  Capture: "The quick-capture window is a single-surface webview with its own realm; its root IS the surface.",
};

type Source = { file: string; source: string };
function read(file: string): Source {
  return { file, source: readFileSync(path.join(REPO_ROOT, file), "utf8") };
}

/** The function (declaration or `const X = (...) =>`) named `name` in `tree`. */
function findComponent(tree: ts.SourceFile, name: string): ts.Node | null {
  let found: ts.Node | null = null;
  const visit = (node: ts.Node) => {
    if (found) return;
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) found = node;
    else if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === name && node.initializer) found = node.initializer;
    else ts.forEachChild(node, visit);
  };
  visit(tree);
  return found;
}

/** Where `name`, used in `from`, is defined: the same file or a relative import. */
function resolveComponent(from: Source, name: string): Source | null {
  const tree = parse(from.file, from.source);
  if (findComponent(tree, name)) return from;
  for (const statement of tree.statements) {
    if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
    const bindings = statement.importClause?.namedBindings;
    if (!bindings || !ts.isNamedImports(bindings) || !bindings.elements.some((el) => el.name.text === name)) continue;
    const base = path.join(path.dirname(from.file), statement.moduleSpecifier.text);
    for (const ext of [".tsx", ".ts"]) {
      if (existsSync(path.join(REPO_ROOT, base + ext))) return read(base + ext);
    }
  }
  return null;
}

/** Unbounded top-level surfaces mounted by component `root` of `from`,
 * following CONTAINERS into the files that define them. */
export function unboundedSurfaces(from: Source, root: string, resolve = resolveComponent): string[] {
  const tree = parse(from.file, from.source);
  const fn = findComponent(tree, root);
  if (!fn) throw new Error(`${from.file}: component ${root} not found`);
  const names = new Set<string>();
  const containers = new Set<string>();
  const mounts = (child: ts.Node) => {
    const name = tagName(child);
    if (name && /^[A-Z]/.test(name)) {
      if (CONTAINERS.has(name)) containers.add(name);
      else if (!APP_SHELLS.has(name)) names.add(name);
    }
    ts.forEachChild(child, mounts);
  };
  mounts(fn);
  const own = [...names].flatMap(name => unboundedMountSites(from.file, from.source, name).map(line => `${from.file} ${name}:${line}`));
  const nested = [...containers].flatMap((name) => {
    const target = resolve(from, name);
    if (!target) throw new Error(`${from.file}: container ${name} not resolvable`);
    return unboundedSurfaces(target, name, resolve);
  });
  return [...own, ...nested];
}

/** Kept for the App-only probes below: the surfaces App itself mounts. */
export function unboundedAppSurfaces(source: string): string[] {
  return unboundedSurfaces({ file: "App.tsx", source }, "App", () => null).map((entry) => entry.slice("App.tsx ".length));
}

function sourceFiles(dir: string): string[] {
  return readdirSync(path.join(REPO_ROOT, dir), { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === "fixtures" || entry.name === "tests" ? [] : sourceFiles(file);
    return /\.tsx$/.test(entry.name) && !/\.(test|spec)\.tsx$/.test(entry.name) ? [file] : [];
  });
}

/** Every Solid window root in the app: `render(() => <Root .../>, mount)`.
 * Derived from the source, so a new window kind cannot escape the scan. */
export function windowRoots(files: Source[]): Array<{ from: Source; root: string }> {
  return files.flatMap((from) => {
    const out: Array<{ from: Source; root: string }> = [];
    const visit = (node: ts.Node) => {
      if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "render") {
        const arg = node.arguments[0];
        if (arg && ts.isArrowFunction(arg)) {
          const name = tagName(ts.isParenthesizedExpression(arg.body) ? arg.body.expression : arg.body);
          if (name && /^[A-Z]/.test(name)) out.push({ from, root: name });
        }
      }
      ts.forEachChild(node, visit);
    };
    visit(parse(from.file, from.source));
    return out;
  });
}

describe("failure-boundary seams (GH #490/#332)", () => {
  it("requires a boundary for every top-level surface of every window root, including new mounts", () => {
    const roots = windowRoots(sourceFiles("src").map(read));
    // Not vacuous: the main app and the workspace window shell are both roots.
    expect(roots.map(({ root }) => root)).toEqual(expect.arrayContaining(["App", "WorkspaceWindowShell"]));
    const failures = roots.filter(({ root }) => !ROOT_EXEMPT[root]).flatMap(({ from, root }) => {
      const owner = resolveComponent(from, root);
      if (!owner) throw new Error(`${from.file}: window root ${root} not resolvable`);
      return unboundedSurfaces(owner, root);
    });
    expect(failures,
      "I-20: independently loaded surfaces own failures; wrap the mount in FailureBoundary. Exemplar: src/App.tsx, QueryExportDialog.").toEqual([]);
  });
  it("follows a container into the file that defines it (review F7 probe)", () => {
    const shell = { file: "src/Shell.tsx", source: 'import { WindowOverlays } from "./Overlays";\nfunction Shell() { return <Show><WindowOverlays /></Show>; }' };
    const overlays = { file: "src/Overlays.tsx", source: 'export function WindowOverlays() {\n  return <><FailureBoundary region="a"><Ok /></FailureBoundary><NewDialog /></>;\n}' };
    expect(unboundedSurfaces(shell, "Shell", (_from, name) => (name === "WindowOverlays" ? overlays : null)))
      .toEqual(["src/Overlays.tsx NewDialog:2"]);
    expect(windowRoots([{ file: "src/w.tsx", source: "render(() => <Shell id={x} />, mount);" }]).map(({ root }) => root)).toEqual(["Shell"]);
  });
  it("detects a newly introduced top-level surface", () => {
    expect(unboundedAppSurfaces("function App() { return <Show><NewPanel /></Show>; }")).toEqual(["NewPanel:1"]);
    expect(unboundedAppSurfaces('function App() { return <FailureBoundary region="New"><NewPanel /></FailureBoundary>; }')).toEqual([]);
  });
  for (const seam of REQUIRED_SEAMS) {
    it(`wraps every <${seam.component}> in ${seam.file}`, () => {
      const source = readFileSync(path.join(REPO_ROOT, seam.file), "utf8");
      expect(mountCount(seam.file, source, seam.component),
        `<${seam.component}> is no longer mounted in ${seam.file}; point its seam at the file that mounts it now.`).toBeGreaterThan(0);
      const mounts = unboundedMountSites(seam.file, source, seam.component);
      expect(
        mounts,
        `<${seam.component}> is mounted outside a <FailureBoundary> at ${seam.file}:${mounts.join(", ")}. `
          + "I-20: a surface owns its failure. A throw there can blank unrelated surfaces (solid-js runUpdates discards the effect "
          + "queue and rethrows with no boundary registered). Wrap it, or argue in the packet why this "
          + "region may take the app down with it. Exemplar: src/components/Page.tsx, Unlinked References.",
      ).toEqual([]);
    });
  }

  it("is not vacuous: it reports an unwrapped mount", () => {
    expect(unboundedMountSites("x.tsx", "const a = () => <LinkedReferences name={n} />;", "LinkedReferences"))
      .toEqual([1]);
  });

  it("accepts a wrapped mount", () => {
    const wrapped = 'const a = () => <FailureBoundary region="R"><LinkedReferences name={n} /></FailureBoundary>;';
    expect(unboundedMountSites("x.tsx", wrapped, "LinkedReferences")).toEqual([]);
  });

  it("leaves prose alone: a comment naming the component is not a mount", () => {
    const prose = "// Rendering <LinkedReferences /> is what this file does.\nconst a = 1;\n";
    expect(unboundedMountSites("x.tsx", prose, "LinkedReferences")).toEqual([]);
  });
});

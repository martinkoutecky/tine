// Which test files can observe what a setup file overrides: the files whose import
// graph reaches it. The CM6 render pass (vitest.cm6.config.ts) uses this to run only
// the files that can mount the editor it forces on, so the list is derived from the
// source, never kept by hand: a new test that mounts an editor is picked up by its
// imports.
import fs from "node:fs";
import path from "node:path";
import ts from "typescript";

const SOURCE_EXTENSIONS = [".ts", ".tsx", ".js", ".mjs", ".jsx"];

function isFile(file) {
  try { return fs.statSync(file).isFile(); } catch { return false; }
}

/** The file a relative specifier names, resolved the way Vite resolves it in this repo
 * (explicit file, implied extension, directory index, `.js` naming a `.ts` source).
 * Bare specifiers (packages) return null: no package imports the app's own modules. */
function resolveRelative(importer, specifier) {
  if (!specifier.startsWith(".")) return null;
  const bare = specifier.replace(/[?#].*$/, "");
  const base = path.resolve(path.dirname(importer), bare);
  const candidates = [base, ...SOURCE_EXTENSIONS.map(ext => base + ext), ...SOURCE_EXTENSIONS.map(ext => path.join(base, "index" + ext))];
  if (/\.[cm]?js$/.test(bare)) candidates.push(...[".ts", ".tsx"].map(ext => base.replace(/\.[cm]?js$/, ext)));
  return candidates.find(isFile) ?? null;
}

/** Runtime import edges of one module: static imports and re-exports, `import("…")`
 * with a literal specifier, and `vi.mock`/`vi.doMock` targets. Type-only imports are
 * erased by the compiler and are not edges. `packages` names the bare imports. */
function moduleEdges(file) {
  const source = ts.createSourceFile(file, fs.readFileSync(file, "utf8"), ts.ScriptTarget.Latest, false, file.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const specifiers = [];
  const visit = node => {
    if (ts.isImportDeclaration(node)) {
      if (!node.importClause?.isTypeOnly) specifiers.push(node.moduleSpecifier.text);
    } else if (ts.isExportDeclaration(node)) {
      if (node.moduleSpecifier && !node.isTypeOnly) specifiers.push(node.moduleSpecifier.text);
    } else if (ts.isCallExpression(node) && node.arguments.length > 0 && ts.isStringLiteralLike(node.arguments[0])) {
      const callee = node.expression;
      const dynamicImport = callee.kind === ts.SyntaxKind.ImportKeyword;
      const viMock = ts.isPropertyAccessExpression(callee) && ts.isIdentifier(callee.expression) && callee.expression.text === "vi"
        && (callee.name.text === "mock" || callee.name.text === "doMock");
      if (dynamicImport || viMock) specifiers.push(node.arguments[0].text);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  const packages = specifiers.filter(spec => !spec.startsWith(".") && !spec.startsWith("node:"))
    .map(spec => spec.split("/").slice(0, spec.startsWith("@") ? 2 : 1).join("/"));
  return { source, edges: specifiers.map(spec => resolveRelative(file, spec)).filter(Boolean), packages };
}

/** Whether an installed package's code reads any of `members`: a text scan of its
 * JavaScript, so a package that measures through a patched prototype method counts. */
function packageReads(root, name, members) {
  const dir = path.join(root, "node_modules", name);
  if (!members.length || !fs.existsSync(dir)) return false;
  return fs.readdirSync(dir, { recursive: true }).some(rel => /\.[cm]?js$/.test(rel)
    && !rel.split(path.sep).includes("node_modules") && isFile(path.join(dir, rel))
    && members.some(member => fs.readFileSync(path.join(dir, rel), "utf8").includes(member)));
}

/** What a setup file and its static runtime imports override: the modules they mock,
 * the `globalThis` properties they assign, and the prototype members they patch. */
function setupOverrides(setupFile) {
  const modules = [];
  const globals = [];
  const members = [];
  const scanned = new Set();
  const scan = file => {
    if (scanned.has(file)) return;
    scanned.add(file);
    const { source } = moduleEdges(file);
    const visit = node => {
      if (ts.isImportDeclaration(node) && !node.importClause?.isTypeOnly) {
        const target = resolveRelative(file, node.moduleSpecifier.text);
        if (target) scan(target);
      }
      if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression) && ts.isIdentifier(node.expression.expression)
        && node.expression.expression.text === "vi" && node.expression.name.text === "mock" && ts.isStringLiteralLike(node.arguments[0])) {
        const target = resolveRelative(file, node.arguments[0].text);
        if (!target) throw new Error(`${file}: cannot resolve vi.mock target ${node.arguments[0].text}`);
        modules.push(target);
      }
      if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken && ts.isPropertyAccessExpression(node.left)) {
        let target = node.left.expression;
        while (ts.isParenthesizedExpression(target) || ts.isAsExpression(target)) target = target.expression;
        if (ts.isIdentifier(target) && target.text === "globalThis") globals.push(node.left.name.text);
        if (ts.isPropertyAccessExpression(target) && target.name.text === "prototype") members.push(node.left.name.text);
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
  };
  scan(setupFile);
  return { modules, globals, members };
}

/** The test files (matched by `include` globs relative to `root`) that can observe what
 * `setupFile` overrides: their import graph reaches a module it mocks, a module that
 * reads a global it sets, or a package whose code reads a prototype member it patches
 * (CodeMirror measures through the Range geometry the CM6 setup supplies, so a test
 * that mounts a CodeMirror view directly is selected). App modules that read a patched
 * member are not seeds: outside a package the patch is layout scaffolding for the
 * editor, and those reads are reached by nearly every render test. Sorted, relative to
 * `root`, with `/` separators (they are vitest globs). */
export function testFilesObservingSetup({ root, include, setupFile }) {
  const tests = [...new Set(include.flatMap(pattern => fs.globSync(pattern, { cwd: root })))]
    .filter(file => !file.split(path.sep).includes("node_modules"))
    .map(file => path.resolve(root, file));
  const overrides = setupOverrides(path.resolve(root, setupFile));
  const edges = new Map();
  const pending = [...tests];
  while (pending.length) {
    const file = pending.pop();
    if (edges.has(file)) continue;
    const { source, edges: next, packages } = moduleEdges(file);
    edges.set(file, { next: [...next, ...packages.map(name => "package:" + name)], text: source.text });
    pending.push(...next);
  }
  const reaches = new Set(overrides.modules);
  const packages = new Set([...edges.values()].flatMap(({ next }) => next.filter(dep => dep.startsWith("package:"))));
  for (const dep of packages) if (packageReads(root, dep.slice("package:".length), overrides.members)) reaches.add(dep);
  for (const [file, { text }] of edges) {
    if (file !== path.resolve(root, setupFile) && overrides.globals.some(name => text.includes(name))) reaches.add(file);
  }
  const importers = new Map();
  for (const [file, { next }] of edges) for (const dep of next) {
    if (!importers.has(dep)) importers.set(dep, []);
    importers.get(dep).push(file);
  }
  const queue = [...reaches];
  while (queue.length) for (const importer of importers.get(queue.pop()) ?? []) {
    if (!reaches.has(importer)) { reaches.add(importer); queue.push(importer); }
  }
  return tests.filter(file => reaches.has(file)).map(file => path.relative(root, file).split(path.sep).join("/")).sort();
}

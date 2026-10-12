#!/usr/bin/env node
// The localization ratchet (tine#107, DESIGN rev. 2 review #4 and #6). Two checks
// over the frontend source, read with the TypeScript parser:
//
// 1. Literal UI text sites: English that reaches the user without the catalog
//    (JSX text, UI attributes, toast arguments, UI-table properties, dialogs). The
//    committed baseline (`src/i18n/literals-baseline.json`) lists the sites that
//    remain, by identity (file, kind, normalized text), not by count: a site not
//    in the baseline fails even if another was removed, and a converted site must
//    leave the baseline (`--update` rewrites it). The list reaching empty, minus a
//    reasoned allowlist, is the proof that English is complete. The identity is
//    (file, kind, owner, text): the owner is the nearest enclosing named function,
//    component, class member or variable (`<module>` if none), so converting one
//    site while adding a same-text site in another owner is one new site, not zero.
//    Scope: src/**/*.ts(x) and index.html (text nodes and the same attribute sinks;
//    script and style bodies are not scanned).
// 2. Load-time resolution: a call of `t`, `render` or `renderEnglish` from src/i18n
//    (named import, or a member of a namespace import) that RUNS AT LOAD resolves
//    once, in whatever locale is active then, and never follows a language change.
//    Tables hold `msg(...)`. No baseline: this always fails. A call runs at load
//    unless its nearest function is not invoked at load (see `atLoadAnalysis`).
//
//   node scripts/i18n-literals.mjs            check (exit 1 on a new site or violation)
//   node scripts/i18n-literals.mjs --update   rewrite the baseline to today's sites
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import ts from "typescript";

const ROOT = path.resolve(import.meta.dirname, "..");
const SRC = path.join(ROOT, "src");
export const BASELINE = path.join(SRC, "i18n/literals-baseline.json");

/** Not UI text the catalog owns: tests, the catalog itself, development fixtures. */
function skipped(rel) {
  return /\.test\.tsx?$/.test(rel)
    || rel.startsWith("src/i18n/")
    || rel === "src/mock.ts"
    || rel.endsWith(".d.ts")
    || rel.endsWith(".gen.ts");
}

const UI_ATTRIBUTES = new Set(["title", "aria-label", "placeholder", "alt", "aria-description", "aria-roledescription"]);
const UI_PROPERTIES = new Set(["label", "title", "description", "details", "hint", "placeholder", "tooltip"]);
const TOAST_CALLS = new Set(["pushToast", "pushToastUnique"]);
const DIALOG_CALLS = new Set(["confirm", "alert", "ask"]);
const RESOLVERS = new Set(["t", "render", "renderEnglish"]);
const CATALOG_CALLS = new Set(["msg", "rawText", "t"]);

/**
 * Text that carries words: at least one run of two letters. The only exemption is
 * "no words": symbols, glyphs, single letters, numbers. There is deliberately NO
 * exemption for identifier-shaped text (`save`, `a/b`): at a UI sink a lower-case
 * word is still English the user reads (DESIGN rev. 2 review #6).
 */
function isWords(text) {
  return /\p{L}{2,}/u.test(text);
}

function normalize(text) {
  return text.replace(/\s+/g, " ").trim();
}

/** The literal text of a string or template expression (`${…}` as `{}`), else null. */
function literalOf(node) {
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return node.text;
  if (ts.isTemplateExpression(node)) return node.head.text + node.templateSpans.map((span) => `{}${span.literal.text}`).join("");
  if (ts.isParenthesizedExpression(node)) return literalOf(node.expression);
  if (ts.isConditionalExpression(node)) {
    const a = literalOf(node.whenTrue);
    const b = literalOf(node.whenFalse);
    return a !== null || b !== null ? `${a ?? "{}"} | ${b ?? "{}"}` : null;
  }
  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken) return literalOf(node.right); // {cond() && "Saved"}
  if (ts.isBinaryExpression(node) && (node.operatorToken.kind === ts.SyntaxKind.BarBarToken || node.operatorToken.kind === ts.SyntaxKind.QuestionQuestionToken)) {
    const a = literalOf(node.left);
    const b = literalOf(node.right);
    return a !== null || b !== null ? `${a ?? "{}"} | ${b ?? "{}"}` : null; // {name() || "Untitled"}
  }
  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.PlusToken) {
    const a = literalOf(node.left);
    const b = literalOf(node.right);
    return a !== null || b !== null ? `${a ?? "{}"}${b ?? "{}"}` : null;
  }
  return null;
}

function calleeName(call) {
  const expression = call.expression;
  if (ts.isIdentifier(expression)) return expression.text;
  if (ts.isPropertyAccessExpression(expression)) return expression.name.text;
  return null;
}

/**
 * Callees whose function arguments are NOT run once, synchronously, at load. Every
 * entry needs a reason; the list is deliberately short. Anything else that takes a
 * function at module level (`map`, `forEach`, `reduce`, `Object.fromEntries(...map)`,
 * `new Promise`, an IIFE, ...) runs it at load.
 */
const DEFERRING_CALLEES = new Map([
  // Solid reactive primitives re-run their function when a dependency changes, and
  // `t()` reads the locale signal, so the result follows a language change.
  ["createMemo", "re-runs when the locale signal changes"],
  ["createEffect", "re-runs when the locale signal changes"],
  ["createRenderEffect", "re-runs when the locale signal changes"],
  ["createComputed", "re-runs when the locale signal changes"],
]);

const transparent = (node) => ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isNonNullExpression(node)
  || ts.isSatisfiesExpression?.(node) || ts.isTypeAssertionExpression(node);

/** The expression's parent, looking through parentheses and type assertions. */
function outerParent(node) {
  let current = node;
  while (current.parent && transparent(current.parent)) current = current.parent;
  return { child: current, parent: current.parent };
}

function className(declaration) {
  if (declaration.name) return declaration.name.text;
  const { parent } = outerParent(declaration);
  return parent && ts.isVariableDeclaration(parent) && ts.isIdentifier(parent.name) ? parent.name.text : null;
}

/** The nearest construct that defers or relocates execution of `node`, else null (module level). */
function boundaryOf(node) {
  for (let current = node.parent; current; current = current.parent) {
    if (ts.isFunctionLike(current) || ts.isClassStaticBlockDeclaration(current) || ts.isPropertyDeclaration(current)) return current;
  }
  return null;
}

/**
 * Does `node` run while the module loads? Rule: the NEAREST function is not invoked
 * at load. Function ancestry alone proves nothing: `ids.map(id => t(id))` at module
 * level has a function ancestor and runs at load.
 *   - no boundary: module level, runs at load;
 *   - class `static` block or `static` field: runs when the class is evaluated;
 *   - instance field or constructor: runs when `new C` runs at load;
 *   - method or accessor: runs when called, never counted (calls go through instances);
 *   - function expression or arrow: an IIFE or an argument of a call that runs at
 *     load (unless the callee defers, see DEFERRING_CALLEES) runs at load; bound to a
 *     variable, it runs at load iff that name is invoked at load; elsewhere (object
 *     member, return value, JSX handler) it is stored unevaluated;
 *   - function declaration: runs at load iff its name is invoked at load.
 * `invoked` is the set of names called, `new`ed, or passed to a load-time call.
 */
function runsAtLoad(node, invoked) {
  const boundary = boundaryOf(node);
  if (!boundary) return true;
  if (ts.isClassStaticBlockDeclaration(boundary)) return runsAtLoad(boundary.parent, invoked);
  if (ts.isPropertyDeclaration(boundary)) {
    const isStatic = boundary.modifiers?.some((m) => m.kind === ts.SyntaxKind.StaticKeyword);
    if (isStatic) return runsAtLoad(boundary.parent, invoked);
    const name = className(boundary.parent);
    return name !== null && invoked.has(name);
  }
  if (ts.isConstructorDeclaration(boundary)) {
    const name = className(boundary.parent);
    return name !== null && invoked.has(name);
  }
  if (ts.isFunctionDeclaration(boundary)) return boundary.name !== undefined && invoked.has(boundary.name.text);
  if (ts.isFunctionExpression(boundary) || ts.isArrowFunction(boundary)) {
    const { child, parent } = outerParent(boundary);
    if (parent && (ts.isCallExpression(parent) || ts.isNewExpression(parent))) {
      if (parent.expression === child) return runsAtLoad(parent, invoked); // IIFE
      if (parent.arguments?.includes(child)) return !DEFERRING_CALLEES.has(calleeName(parent) ?? "") && runsAtLoad(parent, invoked);
    }
    if (parent && ts.isVariableDeclaration(parent) && ts.isIdentifier(parent.name)) return invoked.has(parent.name.text);
    return false;
  }
  return false; // methods, accessors
}

/** Names invoked at load, to a fixpoint: a call at load adds its callee and its identifier arguments. */
function loadInvokedNames(source) {
  const calls = [];
  const collect = (node) => {
    if ((ts.isCallExpression(node) || ts.isNewExpression(node)) && node.expression) calls.push(node);
    ts.forEachChild(node, collect);
  };
  collect(source);
  const invoked = new Set();
  for (let changed = true; changed;) {
    changed = false;
    for (const call of calls) {
      if (!runsAtLoad(call, invoked)) continue;
      const names = [];
      if (ts.isIdentifier(call.expression)) names.push(call.expression.text);
      if (!DEFERRING_CALLEES.has(calleeName(call) ?? "")) {
        for (const argument of call.arguments ?? []) if (ts.isIdentifier(argument)) names.push(argument.text);
      }
      for (const name of names) {
        if (!invoked.has(name)) {
          invoked.add(name);
          changed = true;
        }
      }
    }
  }
  return invoked;
}

/** What this file imports from src/i18n: named bindings, and namespace-import locals. */
function i18nImports(source, rel) {
  const names = [];
  const namespaces = new Set();
  for (const statement of source.statements) {
    if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
    const target = path.normalize(path.join(path.dirname(rel), statement.moduleSpecifier.text));
    if (!/^src\/i18n(?:\/index)?$/.test(target)) continue;
    const bindings = statement.importClause?.namedBindings;
    if (bindings && ts.isNamedImports(bindings)) {
      for (const element of bindings.elements) names.push({ local: element.name.text, imported: (element.propertyName ?? element.name).text });
    } else if (bindings && ts.isNamespaceImport(bindings)) {
      namespaces.add(bindings.name.text);
    }
  }
  return { names, namespaces };
}

/** The nearest enclosing named function, component, class member or variable; `<module>` if none. */
function ownerOf(node) {
  for (let current = node.parent; current; current = current.parent) {
    if ((ts.isFunctionDeclaration(current) || ts.isFunctionExpression(current)) && current.name) return current.name.text;
    if (ts.isClassDeclaration(current) || ts.isClassExpression(current)) {
      const name = className(current);
      if (name) return name;
    }
    if ((ts.isMethodDeclaration(current) || ts.isGetAccessor(current) || ts.isSetAccessor(current) || ts.isPropertyDeclaration(current)) && current.name) {
      const owner = current.parent && (ts.isClassDeclaration(current.parent) || ts.isClassExpression(current.parent)) ? className(current.parent) : null;
      return `${owner ? `${owner}.` : ""}${current.name.getText()}`;
    }
    if (ts.isConstructorDeclaration(current)) return `${className(current.parent) ?? ""}.constructor`;
    if (ts.isVariableDeclaration(current) && ts.isIdentifier(current.name)) return current.name.text;
    if (ts.isPropertyAssignment(current) && current.initializer && ts.isFunctionLike(current.initializer)) return current.name.getText().replace(/["']/g, "");
    if (ts.isExportAssignment(current)) return "default";
  }
  return "<module>";
}

function scanFile(file) {
  return scanSource(path.relative(ROOT, file).split(path.sep).join("/"), fs.readFileSync(file, "utf8"));
}

/** The sites and violations in one source text, as if it lived at `rel` (repo-relative). */
export function scanSource(rel, text) {
  if (rel.endsWith(".html")) return scanHtml(rel, text);
  const source = ts.createSourceFile(rel, text, ts.ScriptTarget.Latest, true, rel.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const sites = [];
  const violations = [];
  const { names: imported, namespaces } = i18nImports(source, rel);
  const resolvers = new Set(imported.filter((name) => RESOLVERS.has(name.imported)).map((name) => name.local));
  const catalog = new Set([...imported.filter((name) => CATALOG_CALLS.has(name.imported)).map((name) => name.local), "msg", "rawText"]);
  const invoked = loadInvokedNames(source);
  const site = (kind, value, node) => {
    const normalized = normalize(value);
    if (!isWords(normalized)) return;
    sites.push({ file: rel, kind, owner: ownerOf(node), text: normalized, line: source.getLineAndCharacterOfPosition(node.getStart()).line + 1 });
  };
  const isCatalogCall = (node) => ts.isCallExpression(node) && catalog.has(calleeName(node) ?? "");
  /** `t(…)` / `render(…)` / `renderEnglish(…)` by a named import, or `ns.t(…)` by a namespace import. */
  const resolverName = (call) => {
    const callee = call.expression;
    if (ts.isIdentifier(callee) && resolvers.has(callee.text)) return callee.text;
    if (ts.isPropertyAccessExpression(callee) && ts.isIdentifier(callee.expression) && namespaces.has(callee.expression.text) && RESOLVERS.has(callee.name.text)) {
      return `${callee.expression.text}.${callee.name.text}`;
    }
    return null;
  };
  const visit = (node) => {
    if (ts.isJsxText(node)) {
      site("jsx-text", node.text, node);
    } else if (ts.isJsxAttribute(node) && UI_ATTRIBUTES.has(node.name.getText(source)) && node.initializer) {
      const init = node.initializer;
      const value = ts.isStringLiteral(init) ? init.text : ts.isJsxExpression(init) && init.expression ? literalOf(init.expression) : null;
      if (value !== null) site(`attr:${node.name.getText(source)}`, value, node);
    } else if (ts.isJsxExpression(node) && node.expression && (ts.isJsxElement(node.parent) || ts.isJsxFragment(node.parent))) {
      const value = literalOf(node.expression);
      if (value !== null) site("jsx-text", value, node);
    } else if (ts.isPropertyAssignment(node) && UI_PROPERTIES.has(node.name.getText(source).replace(/["']/g, ""))) {
      const value = literalOf(node.initializer);
      if (value !== null) site(`prop:${node.name.getText(source).replace(/["']/g, "")}`, value, node);
    } else if (ts.isCallExpression(node)) {
      const name = calleeName(node);
      const first = node.arguments[0];
      if (name && TOAST_CALLS.has(name) && first && !isCatalogCall(first)) {
        // A dynamic argument (`String(error)`) is a site too: its text is its source.
        site("toast", literalOf(first) ?? `{${first.getText(source)}}`, node);
      } else if (name && DIALOG_CALLS.has(name) && first) {
        const value = literalOf(first);
        if (value !== null) site(`dialog:${name}`, value, node);
      }
      const resolver = resolverName(node);
      if (resolver && runsAtLoad(node, invoked)) {
        violations.push(`${rel}:${source.getLineAndCharacterOfPosition(node.getStart()).line + 1}: \`${resolver}(\` runs at load, resolves once in the locale active then, and never follows a language change; store \`msg(...)\` and resolve it where it is shown (a function stored unevaluated, or createMemo/createEffect, is fine)`);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return { sites, violations };
}

let jsdomModule = null;

/** index.html: text nodes and the UI attribute sinks. `<script>` and `<style>` bodies are code, not text. */
export function scanHtml(rel, text) {
  jsdomModule ??= createRequire(import.meta.url)("jsdom");
  const dom = new jsdomModule.JSDOM(text, { includeNodeLocations: true });
  const sites = [];
  const ownerOfElement = (element) => {
    for (let current = element; current; current = current.parentElement) {
      if (current.id) return `#${current.id}`;
    }
    return "<document>";
  };
  const lineOf = (node) => dom.nodeLocation(node)?.startLine ?? 0;
  const walkNode = (node) => {
    if (node.nodeType === 3 /* text */) {
      const tag = node.parentElement?.localName;
      const normalized = normalize(node.data);
      if (tag !== "script" && tag !== "style" && isWords(normalized)) {
        sites.push({ file: rel, kind: "html-text", owner: ownerOfElement(node.parentElement), text: normalized, line: lineOf(node) });
      }
    } else if (node.nodeType === 1 /* element */) {
      for (const attribute of node.attributes) {
        const value = normalize(attribute.value);
        if (UI_ATTRIBUTES.has(attribute.name) && isWords(value)) {
          sites.push({ file: rel, kind: `attr:${attribute.name}`, owner: ownerOfElement(node), text: value, line: lineOf(node) });
        }
      }
    }
    for (const child of node.childNodes) walkNode(child);
  };
  walkNode(dom.window.document.documentElement);
  return { sites, violations: [] };
}

function walk(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full, out);
    else if (/\.tsx?$/.test(entry.name) && !skipped(path.relative(ROOT, full).split(path.sep).join("/"))) out.push(full);
  }
  return out;
}

/** Every literal UI text site and every module-level resolution in src/. */
export function scan(dir = SRC) {
  const sites = [];
  const violations = [];
  for (const file of walk(dir).sort()) {
    const result = scanFile(file);
    sites.push(...result.sites);
    violations.push(...result.violations);
  }
  if (dir === SRC) {
    const result = scanFile(path.join(ROOT, "index.html"));
    sites.push(...result.sites);
    violations.push(...result.violations);
  }
  return { sites, violations };
}

/** A site's identity: file, kind (the sink), owner and text; the line is not part of it. */
export function identity(site) {
  return `${site.file}\t${site.kind}\t${site.owner}\t${site.text}`;
}

export function readBaseline(file = BASELINE) {
  return fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : [];
}

/** New sites (not in the baseline, by multiplicity) and baseline entries now gone. */
export function compare(sites, baseline) {
  const remaining = new Map();
  for (const entry of baseline) remaining.set(entry, (remaining.get(entry) ?? 0) + 1);
  const added = [];
  for (const site of sites) {
    const key = identity(site);
    const count = remaining.get(key) ?? 0;
    if (count > 0) remaining.set(key, count - 1);
    else added.push(site);
  }
  const gone = [...remaining].flatMap(([key, count]) => Array(count).fill(key));
  return { added, gone };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const { sites, violations } = scan();
  if (process.argv.includes("--update")) {
    const entries = sites.map(identity).sort();
    fs.writeFileSync(BASELINE, `${JSON.stringify(entries, null, 0).replace(/^\[/, "[\n").replace(/","/g, "\",\n\"").replace(/\]$/, "\n]")}\n`);
    console.log(`baseline: ${entries.length} literal sites`);
  }
  const { added, gone } = compare(sites, readBaseline());
  for (const violation of violations) console.error(violation);
  for (const site of added) console.error(`${site.file}:${site.line}: new literal UI text (${site.kind}): ${JSON.stringify(site.text)}; add it to the catalog (src/i18n/locales/en) and use t()/msg()`);
  if (gone.length) console.error(`${gone.length} baseline entries were converted or removed; run node scripts/i18n-literals.mjs --update`);
  console.log(`${sites.length} literal sites (baseline ${readBaseline().length})`);
  process.exit(violations.length || added.length || gone.length ? 1 : 0);
}

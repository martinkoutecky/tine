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
//    reasoned allowlist, is the proof that English is complete.
// 2. Module-level resolution: `t(`, `render(` or `renderEnglish(` from src/i18n
//    outside any function body resolves once, at load, in whatever locale is
//    active then, and never follows a language change. Tables hold `msg(...)`.
//    No baseline: this always fails.
//
//   node scripts/i18n-literals.mjs            check (exit 1 on a new site or violation)
//   node scripts/i18n-literals.mjs --update   rewrite the baseline to today's sites
import fs from "node:fs";
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

/** Words, not identifiers or symbols: at least one run of two letters. */
function isWords(text) {
  return /[A-Za-z]{2,}/.test(text) && !/^[a-z][a-zA-Z0-9]*$/.test(text.trim()) && !/^[a-z0-9-]+(?:\/[a-z0-9-]+)+$/.test(text.trim());
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

function insideFunction(node) {
  for (let current = node.parent; current; current = current.parent) {
    if (ts.isFunctionLike(current) || ts.isClassStaticBlockDeclaration(current)) return true;
    // A class property initializer runs per instance, not at load.
    if (ts.isPropertyDeclaration(current) && !current.modifiers?.some((m) => m.kind === ts.SyntaxKind.StaticKeyword)) return true;
  }
  return false;
}

/** Names this file imports from src/i18n (so a local `t` is not mistaken for ours). */
function i18nImports(source, rel) {
  const names = new Set();
  for (const statement of source.statements) {
    if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
    const target = path.normalize(path.join(path.dirname(rel), statement.moduleSpecifier.text));
    if (!/^src\/i18n(?:\/index)?$/.test(target)) continue;
    const bindings = statement.importClause?.namedBindings;
    if (bindings && ts.isNamedImports(bindings)) {
      for (const element of bindings.elements) names.add({ local: element.name.text, imported: (element.propertyName ?? element.name).text });
    }
  }
  return [...names];
}

function scanFile(file) {
  return scanSource(path.relative(ROOT, file).split(path.sep).join("/"), fs.readFileSync(file, "utf8"));
}

/** The sites and violations in one source text, as if it lived at `rel` (repo-relative). */
export function scanSource(rel, text) {
  const source = ts.createSourceFile(rel, text, ts.ScriptTarget.Latest, true, rel.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const sites = [];
  const violations = [];
  const imported = i18nImports(source, rel);
  const resolvers = new Set(imported.filter((name) => RESOLVERS.has(name.imported)).map((name) => name.local));
  const catalog = new Set([...imported.filter((name) => CATALOG_CALLS.has(name.imported)).map((name) => name.local), "msg", "rawText"]);
  const site = (kind, value, node) => {
    const normalized = normalize(value);
    if (!isWords(normalized)) return;
    sites.push({ file: rel, kind, text: normalized, line: source.getLineAndCharacterOfPosition(node.getStart()).line + 1 });
  };
  const isCatalogCall = (node) => ts.isCallExpression(node) && catalog.has(calleeName(node) ?? "");
  const visit = (node) => {
    if (ts.isJsxText(node)) {
      site("jsx-text", node.text, node);
    } else if (ts.isJsxAttribute(node) && UI_ATTRIBUTES.has(node.name.getText(source)) && node.initializer) {
      const init = node.initializer;
      const value = ts.isStringLiteral(init) ? init.text : ts.isJsxExpression(init) && init.expression ? literalOf(init.expression) : null;
      if (value !== null) site(`attr:${node.name.getText(source)}`, value, node);
    } else if (ts.isJsxExpression(node) && node.expression && ts.isJsxElement(node.parent)) {
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
      if (name && resolvers.has(name) && ts.isIdentifier(node.expression) && !insideFunction(node)) {
        violations.push(`${rel}:${source.getLineAndCharacterOfPosition(node.getStart()).line + 1}: \`${name}(\` at module level resolves once, at load, and never follows a language change; store \`msg(...)\` and resolve it where it is shown`);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return { sites, violations };
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
  return { sites, violations };
}

/** A site's identity: file, kind and text; the line is not part of it. */
export function identity(site) {
  return `${site.file}\t${site.kind}\t${site.text}`;
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

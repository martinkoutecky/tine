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

const UI_ATTRIBUTES = new Set(["title", "aria-label", "placeholder", "alt", "aria-description", "aria-roledescription", "aria-valuetext", "label", "hint", "tooltip", "fallback"]);
const UI_PROPERTIES = new Set(["label", "title", "description", "details", "hint", "placeholder", "tooltip", "notice"]);
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
 *   - static method or static accessor: runs at load iff `Class.member` is called (or a
 *     static getter read) at load, or `this.member` is, from a static initializer or a
 *     static method that itself runs at load (see `memberKey`);
 *   - instance method or accessor: runs when called, never counted (calls go through instances);
 *   - function expression or arrow: an IIFE or an argument of a call that runs at
 *     load (unless the callee defers, see DEFERRING_CALLEES) runs at load; bound to a
 *     variable, it runs at load iff that name is invoked at load; elsewhere (object
 *     member, return value, JSX handler) it is stored unevaluated;
 *   - function declaration: runs at load iff its name is invoked at load.
 * `invoked` is the set of names called, `new`ed, or passed to a load-time call, plus the
 * `Class.member` keys of static members called at load.
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
  if (ts.isMethodDeclaration(boundary) || ts.isGetAccessor(boundary) || ts.isSetAccessor(boundary)) {
    const key = isStaticMember(boundary) ? declaredMemberKey(boundary) : null;
    return key !== null && invoked.has(key);
  }
  return false; // anything else
}

const isStaticMember = (member) => member.modifiers?.some((m) => m.kind === ts.SyntaxKind.StaticKeyword) ?? false;

/** The member name as written (`copy`, `"copy"`), or null for a computed name. */
function memberNameOf(name) {
  if (ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNoSubstitutionTemplateLiteral(name)) return name.text;
  return null;
}

/** `Class.member` for a declared class member, or null (anonymous class, computed name). */
function declaredMemberKey(member) {
  const cls = className(member.parent);
  const name = memberNameOf(member.name);
  return cls !== null && name !== null ? `${cls}.${name}` : null;
}

/** The class `this` denotes at `node`, when that is a class evaluated or called statically; arrow functions are transparent. */
function staticThisClass(node) {
  for (let current = node.parent; current; current = current.parent) {
    if (ts.isArrowFunction(current)) continue;
    if (ts.isClassStaticBlockDeclaration(current)) return className(current.parent);
    if ((ts.isPropertyDeclaration(current) || ts.isMethodDeclaration(current) || ts.isGetAccessor(current) || ts.isSetAccessor(current)) && isStaticMember(current)) {
      return className(current.parent);
    }
    if (ts.isFunctionLike(current) || ts.isPropertyDeclaration(current) || ts.isClassLike(current)) return null;
  }
  return null;
}

/** `Class.member`, `this.member` (in a static context) or `Class["member"]` as a key, else null. Single file, no aliases. */
function memberKey(expression) {
  let name = null;
  let target = null;
  if (ts.isPropertyAccessExpression(expression)) {
    name = memberNameOf(expression.name);
    target = expression.expression;
  } else if (ts.isElementAccessExpression(expression) && ts.isStringLiteralLike(expression.argumentExpression)) {
    name = expression.argumentExpression.text;
    target = expression.expression;
  }
  if (name === null || target === null) return null;
  while (transparent(target)) target = target.expression;
  if (ts.isIdentifier(target)) return `${target.text}.${name}`;
  if (target.kind === ts.SyntaxKind.ThisKeyword) {
    const cls = staticThisClass(expression);
    return cls !== null ? `${cls}.${name}` : null;
  }
  return null;
}

/** Names invoked at load, to a fixpoint: a call at load adds its callee and its identifier arguments. */
function loadInvokedNames(source) {
  const calls = [];
  const reads = [];
  const staticGetters = new Set();
  const collect = (node) => {
    if ((ts.isCallExpression(node) || ts.isNewExpression(node)) && node.expression) calls.push(node);
    else if (ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node)) reads.push(node);
    else if (ts.isGetAccessor(node) && isStaticMember(node) && declaredMemberKey(node) !== null) staticGetters.add(declaredMemberKey(node));
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
      else names.push(memberKey(call.expression)); // `Labels.copy()`, `this.copy()`
      if (!DEFERRING_CALLEES.has(calleeName(call) ?? "")) {
        for (const argument of call.arguments ?? []) {
          if (ts.isIdentifier(argument)) names.push(argument.text);
          else names.push(memberKey(argument)); // `ids.map(Labels.copy)`
        }
      }
      for (const name of names) {
        if (name !== null && !invoked.has(name)) {
          invoked.add(name);
          changed = true;
        }
      }
    }
    // A static getter runs when it is read, not called: `export const l = Labels.title`.
    for (const read of reads) {
      const key = memberKey(read);
      if (key === null || invoked.has(key) || !staticGetters.has(key) || !runsAtLoad(read, invoked)) continue;
      invoked.add(key);
      changed = true;
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
  const covered = new Set();
  const calls = [];
  const declarations = new Map();
  const index = (node) => {
    if ((ts.isVariableDeclaration(node) || ts.isFunctionDeclaration(node) || ts.isParameter(node)) && node.name && ts.isIdentifier(node.name)) {
      const entries = declarations.get(node.name.text) ?? [];
      entries.push(node);
      declarations.set(node.name.text, entries);
    }
    if (ts.isCallExpression(node)) calls.push(node);
    ts.forEachChild(node, index);
  };
  index(source);
  const scopeOf = (node) => {
    for (let p = node.parent; p; p = p.parent) if (ts.isBlock(p) || ts.isFunctionLike(p) || ts.isSourceFile(p)) return p;
    return source;
  };
  const resolvedDeclarations = new Map();
  const declarationOf = (node) => {
    if (resolvedDeclarations.has(node)) return resolvedDeclarations.get(node);
    const entries = declarations.get(node.text) ?? [];
    for (let p = node; p; p = p.parent) {
      const found = entries.find(entry => scopeOf(entry) === p);
      if (found) { resolvedDeclarations.set(node, found); return found; }
    }
    resolvedDeclarations.set(node, null);
    return null;
  };
  // Index bindings once rather than searching every call for every displayed
  // parameter/array reference (large components otherwise become quadratic).
  const callsFor = new Map();
  const pushesFor = new Map();
  const remember = (map, key, values) => {
    if (!key) return;
    const entries = map.get(key) ?? [];
    entries.push(...values);
    map.set(key, entries);
  };
  for (const call of calls) {
    if (ts.isIdentifier(call.expression)) remember(callsFor, declarationOf(call.expression), [call]);
    else if (ts.isPropertyAccessExpression(call.expression) && call.expression.name.text === "push" && ts.isIdentifier(call.expression.expression)) {
      remember(pushesFor, declarationOf(call.expression.expression), call.arguments);
    }
  }
  const site = (kind, value, node) => {
    const normalized = normalize(value);
    if (!isWords(normalized)) return;
    sites.push({ file: rel, kind, owner: ownerOf(node), text: normalized, line: source.getLineAndCharacterOfPosition(node.getStart()).line + 1 });
    const mark = (child) => {
      if (ts.isStringLiteralLike(child) || ts.isTemplateExpression(child)) covered.add(child);
      // literalOf(template) reports its static text with {} placeholders,
      // not the values inside those placeholders. Flow must inspect those.
      if (ts.isTemplateExpression(child)) return;
      ts.forEachChild(child, mark);
    };
    mark(node);
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
  const roots = [];
  // Query validation reaches visible alerts through result objects and signal
  // setters, including helpers exported from .ts files (GH #107). Keep syntax
  // fields such as source/dsl and backend diagnostics outside this text sink.
  const queryUi = /^src\/components\/(?:Query\w*|querySheetParts)\.tsx$/.test(rel)
    || rel === "src/editor/searchQuery.ts";
  const visit = (node) => {
    if (ts.isJsxText(node)) {
      site("jsx-text", node.text, node);
    } else if (ts.isJsxAttribute(node) && UI_ATTRIBUTES.has(node.name.getText(source)) && node.initializer) {
      const init = node.initializer;
      const value = ts.isStringLiteral(init) ? init.text : ts.isJsxExpression(init) && init.expression ? literalOf(init.expression) : null;
      if (value !== null) site(`attr:${node.name.getText(source)}`, value, node);
      // A literal branch/template can coexist with helper-produced prose.
      // Keep the direct site's identity, but also trace its dynamic values.
      if (ts.isJsxExpression(init) && init.expression) roots.push(init.expression);
    } else if (ts.isJsxExpression(node) && node.expression && (ts.isJsxElement(node.parent) || ts.isJsxFragment(node.parent))) {
      const value = literalOf(node.expression);
      if (value !== null) site("jsx-text", value, node);
      roots.push(node.expression);
    } else if (ts.isPropertyAssignment(node) && (UI_PROPERTIES.has(node.name.getText(source).replace(/["']/g, ""))
      || queryUi && ["error", "message"].includes(node.name.getText(source).replace(/["']/g, "")))) {
      const value = literalOf(node.initializer);
      if (value !== null) site(`prop:${node.name.getText(source).replace(/["']/g, "")}`, value, node);
      roots.push(node.initializer);
    } else if (ts.isCallExpression(node)) {
      const name = calleeName(node);
      const first = node.arguments[0];
      if (queryUi && ["setError", "setSaveError", "setSaveNotice", "setPreviewError"].includes(name) && first) roots.push(first);
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
  // Follow values, never predicates, keys, event handlers or arbitrary call
  // arguments. A parameter only carries the arguments at its actual call sites.
  // This deliberately remains a single-file analysis; named prose producers are
  // additional roots so exported .ts helpers are checked at their definition.
  const seen = new Map();
  const returned = new Map();
  const returns = (fn, property = null) => {
    const paths = returned.get(fn) ?? new Set();
    if (paths.has(property)) return;
    paths.add(property);
    returned.set(fn, paths);
    if (!fn.body) return;
    if (!ts.isBlock(fn.body)) { flow(fn.body, property); return; }
    const find = (node) => {
      if (ts.isReturnStatement(node) && node.expression) flow(node.expression, property);
      else if (!ts.isFunctionLike(node)) ts.forEachChild(node, find);
    };
    find(fn.body);
  };
  const flow = (node, property = null) => {
    if (!node) return;
    const paths = seen.get(node) ?? new Set();
    if (paths.has(property)) return;
    paths.add(property);
    seen.set(node, paths);
    if (ts.isFunctionLike(node)) returns(node, property);
    else if (ts.isStringLiteralLike(node) || ts.isTemplateExpression(node)) {
      if (!covered.has(node)) site("ui-flow", literalOf(node), node);
      if (ts.isTemplateExpression(node)) for (const span of node.templateSpans) flow(span.expression);
    } else if (transparent(node)) flow(node.expression, property);
    else if (ts.isConditionalExpression(node)) { flow(node.whenTrue, property); flow(node.whenFalse, property); }
    else if (ts.isBinaryExpression(node) && [ts.SyntaxKind.PlusToken, ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken, ts.SyntaxKind.AmpersandAmpersandToken].includes(node.operatorToken.kind)) {
      if (node.operatorToken.kind !== ts.SyntaxKind.AmpersandAmpersandToken) flow(node.left, property);
      flow(node.right, property);
    } else if (ts.isIdentifier(node)) {
      const decl = declarationOf(node);
      if (decl && ts.isParameter(decl)) {
        const fn = decl.parent;
        const pos = fn.parameters.indexOf(decl);
        const binding = ts.isVariableDeclaration(fn.parent) ? fn.parent : fn;
        for (const call of callsFor.get(binding) ?? []) flow(call.arguments[pos], property);
      } else if (decl && ts.isVariableDeclaration(decl)) {
        flow(decl.initializer, property);
        for (const argument of pushesFor.get(decl) ?? []) flow(argument, property);
      }
    } else if (ts.isCallExpression(node) && !isCatalogCall(node) && !resolverName(node)) {
      if (ts.isIdentifier(node.expression)) {
        const decl = declarationOf(node.expression);
        const fn = decl && ts.isVariableDeclaration(decl) ? decl.initializer : decl;
        if (fn && ts.isFunctionLike(fn)) returns(fn, property);
        else if (fn && ts.isCallExpression(fn)) flow(fn, property);
        else if (DEFERRING_CALLEES.has(node.expression.text) && node.arguments[0] && ts.isFunctionLike(node.arguments[0])) returns(node.arguments[0], property);
      } else if (ts.isPropertyAccessExpression(node.expression) && ["join", "trim", "toLowerCase", "toUpperCase", "replace", "map", "flatMap", "filter", "slice"].includes(node.expression.name.text)) {
        flow(node.expression.expression);
        if (node.expression.name.text === "join") flow(node.arguments[0]);
        if (["map", "flatMap"].includes(node.expression.name.text) && node.arguments[0] && ts.isFunctionLike(node.arguments[0])) returns(node.arguments[0]);
      }
    } else if (ts.isArrayLiteralExpression(node)) node.elements.forEach(item => flow(item, property));
    else if (ts.isSpreadElement(node)) flow(node.expression);
    else if (ts.isObjectLiteralExpression(node)) {
      for (const prop of node.properties) {
        const name = prop.name?.getText(source).replace(/["']/g, "");
        if (property ? name !== property : ![...UI_PROPERTIES, "text"].includes(name)) continue;
        if (ts.isPropertyAssignment(prop)) flow(prop.initializer);
        else if (ts.isShorthandPropertyAssignment(prop)) flow(prop.name);
      }
    } else if (ts.isElementAccessExpression(node) || ts.isPropertyAccessExpression(node)) {
      if (ts.isPropertyAccessExpression(node)) flow(node.expression, node.name.text);
      // Constant vocabulary tables (ATTR_PHRASE[leaf.attr], etc.). Only the
      // values are displayed; the table's keys and the lookup are syntax.
      if (ts.isIdentifier(node.expression)) {
        const decl = declarationOf(node.expression);
        if (decl && ts.isVariableDeclaration(decl) && decl.initializer && ts.isObjectLiteralExpression(decl.initializer)) {
          const key = ts.isPropertyAccessExpression(node) ? node.name.text : ts.isStringLiteralLike(node.argumentExpression) ? node.argumentExpression.text : null;
          for (const prop of decl.initializer.properties) if (ts.isPropertyAssignment(prop) && (!key || prop.name.getText(source).replace(/["']/g, "") === key)) flow(prop.initializer);
        }
      }
    }
  };
  for (const [name, entries] of declarations) if (/(?:Sentence|Phrase|Label|Title|Words)$/.test(name)) {
    for (const decl of entries) {
      const fn = ts.isVariableDeclaration(decl) ? decl.initializer : decl;
      if (fn && ts.isFunctionLike(fn)) returns(fn);
    }
  }
  roots.forEach(root => flow(root));
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
  return baselineEntries(file).map(entry => typeof entry === "string" ? entry : entry.site);
}

function baselineEntries(file = BASELINE) {
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
    // Preserve the justification attached to intentionally untranslated sites.
    const retained = new Map(baselineEntries().filter(entry => typeof entry !== "string").map(entry => [entry.site, entry]));
    const entries = sites.map(identity).sort().map(key => retained.get(key) ?? key);
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

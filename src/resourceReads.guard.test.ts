import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

// Reading a Solid resource is how this app throws into render.
//
// `read()` (solid-js 1.9.13, dist/solid.js:318-322) rethrows the rejection, and
// so does the `latest` getter (:390-397). `runUpdates` discards the whole
// pending effect queue before `handleError` runs, so ONE failed fetch used to
// blank the window — GH #490 in the conflict panel, GH #332 in the field.
// `FailureBoundary` caught that; this ratchet stops it happening.
//
// The rule: a resource BINDING is never called. Read it through `readOr` /
// `readLatestOr` (src/resourceRead.ts), which consult `resource.error` first
// and hand back the fallback the site already had — every one of these sites
// had written one, and none of those branches could run. `.error`, `.loading`,
// `.state` and `.refetch` stay available, because none of them throws.
//
// Exemplar to imitate: src/render/inline.tsx (preview, grp).
const FACTORIES = new Set(["createResource"]);
const READERS = new Set(["readOr", "readLatestOr"]);
/** Properties that do not throw. `latest` is NOT one of them (dist/solid.js:394). */
const SAFE_PROPERTIES = new Set(["error", "loading", "state", "refetch", "mutate"]);

const SRC = path.resolve(__dirname);

/** Files whose reads are not converted yet because another og lane owns them
 *  while it is in flight (og-D, 2026-09-29). This list may only SHRINK: the
 *  "still offends" test below fails the moment one of these files is clean, so
 *  its entry must then be deleted, and a file cannot be added. Owner lane in the
 *  value; the conversion is mechanical (see src/render/inline.tsx). */
const PENDING_OWNED_FILES: Record<string, string> = {
  "components/Macro.tsx": "OG-E (query tail)",
  "components/QueryBuilder.tsx": "OG-E (query tail)",
  "components/QueryExportDialog.tsx": "OG-E (query tail)",
  "components/QueryWorkspace.tsx": "OG-E (query tail)",
  "components/querySheetParts.tsx": "OG-E (query tail)",
};

function sources(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = path.join(dir, entry);
    if (statSync(full).isDirectory()) sources(full, out);
    else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry)) out.push(full);
  }
  return out;
}

function parse(file: string, source: string): ts.SourceFile {
  return ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
}

/** The nearest enclosing function (or the file), which is where a `const`
 *  binding is visible. Bindings are scoped, not file-wide: `Page.tsx` has a
 *  resource named `groups` AND a plain parameter named `groups`, and a file-wide
 *  name set reports the parameter as a resource read. */
function scopeOf(node: ts.Node): ts.Node {
  let current: ts.Node | undefined = node.parent;
  while (current) {
    if (ts.isFunctionDeclaration(current) || ts.isFunctionExpression(current)
        || ts.isArrowFunction(current) || ts.isMethodDeclaration(current)
        || ts.isSourceFile(current)) return current;
    current = current.parent;
  }
  return node.getSourceFile();
}

function encloses(scope: ts.Node, node: ts.Node): boolean {
  for (let current: ts.Node | undefined = node; current; current = current.parent) {
    if (current === scope) return true;
  }
  return false;
}

interface Binding { name: string; scope: ts.Node }

/** Names bound to a resource by one of the factories, with the scope each is
 *  visible in. */
export function resourceBindings(file: string, source: string, parsed?: ts.SourceFile): Binding[] {
  // The caller may pass the tree it will scan. Scopes are compared by node
  // IDENTITY, so a second `parse()` of the same text yields nodes that match
  // nothing and quietly makes the whole scan vacuous — the two self-checks
  // below are what caught exactly that.
  const sourceFile = parsed ?? parse(file, source);
  const bindings: Binding[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node)
        && FACTORIES.has(node.expression.getText(sourceFile).replace(/<.*$/s, ""))) {
      let declaration: ts.Node | undefined = node.parent;
      while (declaration && !ts.isVariableDeclaration(declaration)) declaration = declaration.parent;
      if (declaration && ts.isVariableDeclaration(declaration)) {
        const bound = declaration.name;
        const scope = scopeOf(declaration);
        if (ts.isArrayBindingPattern(bound)) {
          const first = bound.elements[0];
          if (first && ts.isBindingElement(first) && ts.isIdentifier(first.name)) {
            bindings.push({ name: first.name.text, scope });
          }
        } else if (ts.isIdentifier(bound)) bindings.push({ name: bound.text, scope });
      }
    }
    ts.forEachChild(node, visit);
  };
  ts.forEachChild(sourceFile, visit);
  return bindings;
}

/** Places where a resource binding is read in a way that can throw. */
export function throwingReads(file: string, source: string): string[] {
  const sourceFile = parse(file, source);
  const bindings = resourceBindings(file, source, sourceFile);
  if (bindings.length === 0) return [];
  const offending: string[] = [];
  const at = (node: ts.Node) => sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1;
  const isResource = (name: ts.Identifier, node: ts.Node) =>
    bindings.some((binding) => binding.name === name.text && encloses(binding.scope, node));
  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)
        && node.arguments.length === 0 && isResource(node.expression, node)) {
      offending.push(`${node.expression.text}()@${at(node)}`);
    }
    if (ts.isPropertyAccessExpression(node)
        && ts.isIdentifier(node.expression)
        && !SAFE_PROPERTIES.has(node.name.text)
        && isResource(node.expression, node)) {
      offending.push(`${node.expression.text}.${node.name.text}@${at(node)}`);
    }
    ts.forEachChild(node, visit);
  };
  ts.forEachChild(sourceFile, visit);
  return offending;
}

const RULE = "A Solid resource binding must not be called: reading a rejected resource THROWS "
  + "(solid.js read() :318-322, latest :390-397) and the throw discards the whole pending effect "
  + "queue, which is how one failed fetch blanks a region. Read it with readOr/readLatestOr "
  + "(src/resourceRead.ts) and give it the fallback this site already draws for 'not loaded'. "
  + "Exemplar: src/render/inline.tsx. If emptiness would MISINFORM the user, "
  + "pair it with <ResourceFailure of={…} /> (exemplar: src/components/ResourceFailure.tsx).";

describe("resource reads never throw into render (GH #490/#332)", () => {
  const files = sources(SRC);

  it("finds the resources it is supposed to police", () => {
    const counted = files.reduce((total, file) => total + resourceBindings(file, readFileSync(file, "utf8")).length, 0);
    expect(counted).toBeGreaterThan(25);
  });

  it("calls no resource binding anywhere in src/ outside the shrinking pending list", () => {
    const offenders: string[] = [];
    for (const file of files) {
      const relative = path.relative(SRC, file);
      if (relative in PENDING_OWNED_FILES) continue;
      const found = throwingReads(file, readFileSync(file, "utf8"));
      if (found.length) offenders.push(`${relative}: ${found.join(", ")}`);
    }
    expect(offenders, RULE).toEqual([]);
  });

  it("keeps the pending list honest: every listed file still offends (delete an entry once converted)", () => {
    for (const relative of Object.keys(PENDING_OWNED_FILES)) {
      const file = path.join(SRC, relative);
      expect(throwingReads(file, readFileSync(file, "utf8")).length, `${relative} is clean: remove it from PENDING_OWNED_FILES`).toBeGreaterThan(0);
    }
  });

  it("is not vacuous: a called binding is reported", () => {
    const source = "const [data] = createResource(load);\nconst x = data();\n";
    expect(throwingReads("x.tsx", source)).toEqual(["data()@2"]);
  });

  it("reports .latest, which throws exactly as the call does", () => {
    const source = "const [data] = createResource(load);\nconst x = data.latest;\n";
    expect(throwingReads("x.tsx", source)).toEqual(["data.latest@2"]);
  });

  it("accepts readOr, and the properties that cannot throw", () => {
    const source = "const [data, { refetch }] = createResource(load);\n"
      + 'const x = () => readOr(data, undefined, "x");\n'
      + "const busy = () => data.loading || data.error !== undefined;\n";
    expect(throwingReads("x.tsx", source)).toEqual([]);
  });

  it("scopes a binding to its own function, so a same-named parameter is not a read", () => {
    const source = "function count(groups) { return groups()?.length; }\n"
      + "function Panel() { const [groups] = createResource(load); return readOr(groups, undefined, \"g\"); }\n";
    expect(throwingReads("x.tsx", source)).toEqual([]);
  });

  it("leaves a non-resource accessor alone", () => {
    const source = "const [count, setCount] = createSignal(0);\nconst x = count();\n";
    expect(throwingReads("x.tsx", source)).toEqual([]);
  });

  it("knows readOr and readLatestOr are the two sanctioned readers", () => {
    expect([...READERS].sort()).toEqual(["readLatestOr", "readOr"]);
  });
});

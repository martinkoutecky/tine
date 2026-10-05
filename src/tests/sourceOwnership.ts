import { readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";

// Dependency-ownership checks for the architecture guards. A guard that asks
// "does this file contain the text `import { x }`" fails an equivalent alias,
// a multi-name import or a reflow, and passes a file that spells the text in a
// comment. These helpers read the TypeScript AST instead: they answer WHO a
// file gets a computation from (an import, resolved to a repo path) and whether
// it calls it or declares its own copy. Exemplar: src/ogDupal1.guard.test.ts.

export interface ImportBinding {
  local: string;
  /** The exported name (`default` for a default import, `*` for a namespace). */
  imported: string;
  /** Repo-relative module path without a .ts/.tsx/.js extension, or the bare specifier. */
  module: string;
}

/** `source` overrides the file read: planted-violation tests pass it. */
export function parseSource(file: string, source?: string): ts.SourceFile {
  return ts.createSourceFile(
    file, source ?? readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
}

function resolveModule(file: string, specifier: string): string {
  const target = specifier.startsWith(".")
    ? path.posix.normalize(path.posix.join(path.posix.dirname(file), specifier))
    : specifier;
  return target.replace(/\.(?:tsx?|js)$/, "");
}

export function importBindings(file: string, source?: string): ImportBinding[] {
  const sf = parseSource(file, source);
  const out: ImportBinding[] = [];
  for (const statement of sf.statements) {
    if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
    const module = resolveModule(file, statement.moduleSpecifier.text);
    const clause = statement.importClause;
    if (!clause) continue;
    if (clause.name) out.push({ local: clause.name.text, imported: "default", module });
    const bindings = clause.namedBindings;
    if (bindings && ts.isNamespaceImport(bindings)) out.push({ local: bindings.name.text, imported: "*", module });
    if (bindings && ts.isNamedImports(bindings)) {
      for (const element of bindings.elements) {
        out.push({ local: element.name.text, imported: (element.propertyName ?? element.name).text, module });
      }
    }
  }
  return out;
}

/** Does `file` import `imported` from the repo module `module` (any alias, any sibling names)? */
export function importsFrom(file: string, imported: string, module: string, source?: string): boolean {
  return importBindings(file, source).some((b) => b.imported === imported && b.module === module);
}

function functionBody(sf: ts.SourceFile, name: string): ts.Node | undefined {
  let found: ts.Node | undefined;
  const visit = (node: ts.Node) => {
    if (found) return;
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) found = node;
    else if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === name &&
      node.initializer && (ts.isArrowFunction(node.initializer) || ts.isFunctionExpression(node.initializer))) found = node;
    else ts.forEachChild(node, visit);
  };
  visit(sf);
  return found;
}

/** Does `file` declare its own function (declaration or const arrow/function expression) called `name`? */
export function declaresFunction(file: string, name: string, source?: string): boolean {
  return functionBody(parseSource(file, source), name) !== undefined;
}

/** Does `file` declare a variable whose name matches `name` and whose initializer is an array literal? */
export function declaresArrayConstant(file: string, name: RegExp, source?: string): boolean {
  let found = false;
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && name.test(node.name.text) &&
      node.initializer && ts.isArrayLiteralExpression(node.initializer)) found = true;
    ts.forEachChild(node, visit);
  };
  visit(parseSource(file, source));
  return found;
}

/** Does `file` call the binding it imports as `imported` from `module`, under any local alias?
 *  With `within`, only calls inside that named function count. */
export function callsImported(file: string, imported: string, module: string, within?: string, source?: string): boolean {
  const locals = new Set(importBindings(file, source).filter((b) => b.imported === imported && b.module === module).map((b) => b.local));
  if (locals.size === 0) return false;
  const sf = parseSource(file, source);
  const scope = within === undefined ? sf : functionBody(sf, within);
  if (!scope) return false;
  let found = false;
  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && locals.has(node.expression.text)) found = true;
    ts.forEachChild(node, visit);
  };
  visit(scope);
  return found;
}

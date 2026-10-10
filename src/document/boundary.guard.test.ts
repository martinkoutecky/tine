import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { expect, it } from "vitest";

type Sources = Map<string, string>;
const DOC = "src/document";

function sourceFiles(dir = "src"): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.posix.join(dir, entry.name);
    return entry.isDirectory() ? sourceFiles(file)
      : /\.[jt]sx?$/.test(file) && !file.includes(".test.") && !file.endsWith(".d.ts") ? [file] : [];
  });
}

function sources(): Sources {
  return new Map(sourceFiles().map((file) => [file, readFileSync(file, "utf8")]));
}

// Each check parses every production source; under a loaded full-suite run
// that exceeds vitest's 5 s default, so scans get a budget, not an assertion.
const SCAN_TIMEOUT_MS = 30_000;

function production(file: string): boolean {
  return !file.startsWith(`${DOC}/`) && !file.endsWith("/mock.ts");
}

function imports(file: string, source: string): { spec: string; names: string[]; typeOnly: boolean }[] {
  const parsed = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
  return parsed.statements.flatMap((statement) => {
    if (!ts.isImportDeclaration(statement) && !ts.isExportDeclaration(statement)) return [];
    const module = statement.moduleSpecifier;
    if (!module || !ts.isStringLiteral(module)) return [];
    const clause = ts.isImportDeclaration(statement) ? statement.importClause : null;
    const bindings = clause?.namedBindings;
    const names = bindings && ts.isNamedImports(bindings)
      ? bindings.elements.map((entry) => entry.propertyName?.text ?? entry.name.text) : [];
    const typeOnly = ts.isExportDeclaration(statement) ? statement.isTypeOnly
      : !!clause?.isTypeOnly || !!(bindings && ts.isNamedImports(bindings)
        && bindings.elements.length && bindings.elements.every((entry) => entry.isTypeOnly));
    return [{ spec: module.text, names, typeOnly }];
  });
}

function resolved(file: string, spec: string, all: Sources): string | null {
  if (!spec.startsWith(".")) return null;
  const base = path.posix.normalize(path.posix.join(path.posix.dirname(file), spec));
  return [`${base}.ts`, `${base}.tsx`, `${base}/index.ts`, `${base}/index.tsx`]
    .find((candidate) => all.has(candidate)) ?? null;
}

function directImportViolations(all: Sources): string[] {
  const bad: string[] = [];
  for (const [file, source] of all) {
    if (!production(file)) continue;
    for (const entry of imports(file, source)) {
      const target = resolved(file, entry.spec, all);
      if (target?.startsWith(`${DOC}/`) && target !== `${DOC}/index.ts`)
        bad.push(`I-11: import the document folder index only; ${file} imports ${target}; exemplar src/components/PageProps.tsx`);
    }
  }
  return bad;
}

function containerImportViolations(all: Sources): string[] {
  const bad: string[] = [];
  for (const [file, source] of all) {
    if (!production(file)) continue;
    for (const entry of imports(file, source)) {
      const target = resolved(file, entry.spec, all);
      if (!target?.startsWith(`${DOC}/`)) continue;
      for (const name of entry.names.filter((name) => name === "doc" || name === "setDoc"))
        bad.push(`I-11: no doc/setDoc import outside document; ${file} imports ${name}; exemplar src/components/PageProps.tsx`);
    }
  }
  return bad;
}

// Step 3b P2a: the page-host client is reviewed and tested before it is wired;
// old persistence stays the only page authority until the P2b flip, which
// deletes this check in the same commit that imports the client.
const HOST_CLIENT = `${DOC}/host/`;
function unwiredClientViolations(all: Sources): string[] {
  const bad: string[] = [];
  for (const [file, source] of all) {
    if (file.startsWith(HOST_CLIENT) || file.endsWith("/mock.ts")) continue;
    for (const entry of imports(file, source)) {
      if (resolved(file, entry.spec, all)?.startsWith(HOST_CLIENT))
        bad.push(`P2a client is unwired: ${file} imports ${entry.spec}; old persistence is the only authority until P2b; exemplar src/document/save/engine.ts`);
    }
  }
  return bad;
}

function backendWriteViolations(all: Sources): string[] {
  const bad: string[] = [];
  const directKinds: Record<string, { index: number; kinds: string[] }> = {
    renamePage: { index: 2, kinds: ["rename-page"] },
    copyGuideIntoGraph: { index: 1, kinds: ["replace-page"] },
    restoreBackup: { index: 1, kinds: ["replace-page"] },
    trashJournalFile: { index: 1, kinds: ["delete-page"] },
    mergePages: { index: 2, kinds: ["insert-blocks", "delete-page"] },
    renameFileToPage: { index: 2, kinds: ["rename-page"] },
    trashSyncConflict: { index: 1, kinds: ["delete-page"] },
    resolveSyncConflict: { index: 5, kinds: ["replace-page", "delete-page"] },
    writeHighlights: { index: 4, kinds: ["replace-page"] },
    setJournalTitleFormat: { index: 1, kinds: ["rename-page"] },
  };
  for (const [file, source] of all) {
    if (file.endsWith("/mock.ts")) continue;
    const parsed = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
    const visit = (node: ts.Node): void => {
      if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression)
          && ts.isCallExpression(node.expression.expression)
          && node.expression.expression.expression.getText(parsed) === "backend") {
        const method = node.expression.name.text;
        if (method === "savePages" || method === "deletePage") {
          if (file !== `${DOC}/save/engine.ts`)
            bad.push(`I-1: backend page writes belong in document/save/engine.ts; OG-RULES Rule 8; ${file}; exemplar src/document/save/engine.ts`);
        } else if (directKinds[method]) {
          const { index, kinds } = directKinds[method];
          const argument = node.arguments[index];
          const declared = argument && ts.isStringLiteral(argument) ? [argument.text]
            : argument && ts.isArrayLiteralExpression(argument) && argument.elements.every(ts.isStringLiteral)
              ? argument.elements.map((element) => (element as ts.StringLiteral).text) : [];
          if (JSON.stringify(declared) !== JSON.stringify(kinds))
            bad.push(`I-1: backend page writer ${method} must declare ${kinds.join("+")} (OG-RULES Rule 8); ${file}; exemplar src/components/Settings.tsx`);
        }
      }
      ts.forEachChild(node, visit);
    };
    visit(parsed);
  }
  return bad;
}

function dtoBuilderViolations(all: Sources): string[] {
  const bad: string[] = [];
  for (const [file, source] of all) {
    if (file === `${DOC}/convert.ts` || file.endsWith("/mock.ts")) continue;
    const parsed = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
    const visit = (node: ts.Node): void => {
      if (ts.isObjectLiteralExpression(node)) {
        const names = new Set(node.properties.flatMap((prop) => prop.name && ts.isIdentifier(prop.name) ? [prop.name.text] : []));
        if (names.has("pre_block") && names.has("blocks"))
          bad.push(`I-12: build PageDto in document/convert.ts; ${file}; exemplar src/document/convert.ts`);
      }
      if (ts.isVariableDeclaration(node) && node.type?.getText(parsed) === "PageDto")
        bad.push(`I-12: PageDto-typed value outside convert.ts; ${file}; exemplar src/document/convert.ts`);
      ts.forEachChild(node, visit);
    };
    visit(parsed);
  }
  return bad;
}

function documentCycle(all: Sources): string[] | null {
  const graph = new Map<string, Set<string>>();
  const collapse = (file: string) => file.startsWith(`${DOC}/`) ? DOC : file;
  for (const [file, source] of all) {
    const from = collapse(file);
    if (!graph.has(from)) graph.set(from, new Set());
    for (const entry of imports(file, source)) {
      if (entry.typeOnly) continue;
      const target = resolved(file, entry.spec, all);
      if (target && collapse(target) !== from) graph.get(from)!.add(collapse(target));
    }
  }
  const search = (at: string, seen: Set<string>, trail: string[]): string[] | null => {
    if (at === DOC) return trail;
    if (seen.has(at)) return null;
    seen.add(at);
    for (const next of graph.get(at) ?? []) {
      const found = search(next, seen, [...trail, next]);
      if (found) return found;
    }
    return null;
  };
  for (const neighbor of graph.get(DOC) ?? []) {
    const found = search(neighbor, new Set(), [DOC, neighbor]);
    if (found) return found;
  }
  return null;
}

function assertNoDocumentCycle(all: Sources): void {
  const cycle = documentCycle(all);
  if (cycle) throw new Error(`I-11: document must not join an import cycle; exemplar src/components/PageProps.tsx; ${cycle.join(" -> ")}`);
}

it("I-11 document imports use the folder index", () => {
  const all = sources();
  expect(directImportViolations(all)).toEqual([]);
  all.set("src/__plant.ts", 'import { doc } from "./document/model";');
  expect(directImportViolations(all)[0]).toContain("I-11: import the document folder index only");
}, SCAN_TIMEOUT_MS);

it("I-11 document containers stay private", () => {
  const all = sources();
  expect(containerImportViolations(all)).toEqual([]);
  all.set("src/__plant.ts", 'import { setDoc } from "./document";');
  expect(containerImportViolations(all)[0]).toContain("I-11: no doc/setDoc import");
}, SCAN_TIMEOUT_MS);

it("I-1 backend page writes use the engine", () => {
  const all = sources();
  expect(backendWriteViolations(all)).toEqual([]);
  all.set("src/__plant.ts", "backend().savePages('id', page, null, false)");
  expect(backendWriteViolations(all)[0]).toContain("I-1: backend page writes belong");
  all.set("src/__plant.ts", 'backend().restoreBackup("stamp")');
  expect(backendWriteViolations(all)[0]).toContain("OG-RULES Rule 8");
}, SCAN_TIMEOUT_MS);

it("I-12 PageDto construction stays in convert", () => {
  const all = sources();
  expect(dtoBuilderViolations(all)).toEqual([]);
  all.set("src/__plant.ts", "const x = { pre_block: null, blocks: [] };");
  expect(dtoBuilderViolations(all)[0]).toContain("I-12: build PageDto");
}, SCAN_TIMEOUT_MS);

it("I-11 the document module has no import cycle", () => {
  const all = sources();
  assertNoDocumentCycle(all);
  all.set("src/document/__plant.ts", 'import "../__plant";');
  all.set("src/__plant.ts", 'import "./document";');
  expect(() => assertNoDocumentCycle(all)).toThrow("I-11: document must not join an import cycle; exemplar src/components/PageProps.tsx; src/document -> src/__plant.ts -> src/document");
}, SCAN_TIMEOUT_MS);

it("P2a client is unwired: no production module imports the page-host client", () => {
  const all = sources();
  expect(unwiredClientViolations(all)).toEqual([]);
  all.set("src/document/__plant.ts", 'import { HostClient } from "./host/client";');
  expect(unwiredClientViolations(all)[0]).toContain("P2a client is unwired: src/document/__plant.ts");
  all.delete("src/document/__plant.ts");
  all.set("src/__plant.ts", 'export { settle } from "./document/host/settle";');
  expect(unwiredClientViolations(all)[0]).toContain("P2a client is unwired: src/__plant.ts");
}, SCAN_TIMEOUT_MS);

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

function backendWriteViolations(all: Sources): string[] {
  const bad: string[] = [];
  for (const [file, source] of all) {
    if (file.endsWith("/mock.ts") || file === `${DOC}/save/engine.ts`) continue;
    if (/backend\(\)\.(?:savePage|deletePage)\s*\(/.test(source))
      bad.push(`I-1: backend page writes belong in document/save/engine.ts; ${file}; exemplar src/document/save/engine.ts`);
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
});

it("I-11 document containers stay private", () => {
  const all = sources();
  expect(containerImportViolations(all)).toEqual([]);
  all.set("src/__plant.ts", 'import { setDoc } from "./document";');
  expect(containerImportViolations(all)[0]).toContain("I-11: no doc/setDoc import");
});

it("I-1 backend page writes use the engine", () => {
  const all = sources();
  expect(backendWriteViolations(all)).toEqual([]);
  all.set("src/__plant.ts", "backend().savePage('id', page, null, false)");
  expect(backendWriteViolations(all)[0]).toContain("I-1: backend page writes belong");
});

it("I-12 PageDto construction stays in convert", () => {
  const all = sources();
  expect(dtoBuilderViolations(all)).toEqual([]);
  all.set("src/__plant.ts", "const x = { pre_block: null, blocks: [] };");
  expect(dtoBuilderViolations(all)[0]).toContain("I-12: build PageDto");
});

it("I-11 the document module has no import cycle", () => {
  const all = sources();
  assertNoDocumentCycle(all);
  all.set("src/document/__plant.ts", 'import "../__plant";');
  all.set("src/__plant.ts", 'import "./document";');
  expect(() => assertNoDocumentCycle(all)).toThrow("I-11: document must not join an import cycle; exemplar src/components/PageProps.tsx; src/document -> src/__plant.ts -> src/document");
});

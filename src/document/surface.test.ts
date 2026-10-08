import { readFileSync } from "node:fs";
import ts from "typescript";
import { expect, it } from "vitest";

const RULE = "I-11: index.ts exports exactly SURFACE.txt; additions after batch 02b require '# why: <reason; batch>'";

function checkSurface(source: string, lines: string[], legacy: string[]): void {
  const file = ts.createSourceFile("index.ts", source, ts.ScriptTarget.Latest, true);
  const declarations = file.statements.filter(ts.isExportDeclaration);
  if (!declarations.every((statement) => statement.exportClause && ts.isNamedExports(statement.exportClause))) throw new Error(RULE);
  if (file.statements.some((statement) => ts.isExportAssignment(statement) ||
    (!ts.isExportDeclaration(statement) && ts.canHaveModifiers(statement) &&
      ts.getModifiers(statement)?.some((modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword)))) throw new Error(RULE);
  const actual = file.statements.flatMap((statement) =>
    ts.isExportDeclaration(statement) && statement.exportClause && ts.isNamedExports(statement.exportClause)
      ? statement.exportClause.elements.map((element) => element.name.text)
      : []
  );
  const entries = lines.map((line) => /^(\w+)(?:  # why: (.+))?$/.exec(line));
  if (entries.some((entry) => !entry)) throw new Error(RULE);
  if (entries.some((entry) => entry?.[2] && !/^.+;\s*batch\S*$/.test(entry[2]))) throw new Error(RULE);
  const listed = entries.map((entry) => entry![1]);
  if (listed.some((name, i) => !legacy.includes(name) && !entries[i]![2])) throw new Error(RULE);
  if (listed.some((name, i) => legacy.includes(name) && entries[i]![2])) throw new Error(RULE);
  if (listed.join("\n") !== [...new Set(listed)].sort().join("\n") || actual.sort().join("\n") !== listed.join("\n"))
    throw new Error(RULE);
}

const source = () => readFileSync("src/document/index.ts", "utf8");
const lines = () => readFileSync("src/document/SURFACE.txt", "utf8").trim().split("\n");
const legacy = () => readFileSync("src/document/SURFACE_BASE.txt", "utf8").trim().split("\n");

it("pins the document public surface", () => {
  checkSurface(source(), lines(), legacy());
});

it("rejects a planted unlisted export", () => {
  expect(() => checkSurface(source() + '\nexport { planted } from "./model";\n', lines(), legacy())).toThrow(RULE);
});

it("rejects a listed name missing from index", () => {
  expect(() => checkSurface(source(), [...lines(), "planted  # why: planted; batch-test"].sort(), legacy())).toThrow(RULE);
});

it("rejects a new line without its justification", () => {
  expect(() => checkSurface(source() + '\nexport { planted } from "./model";\n', [...lines(), "planted"].sort(), legacy())).toThrow(RULE);
});

it("permits a justified new export", () => {
  expect(() => checkSurface(source() + '\nexport { planted } from "./model";\n', [...lines(), "planted  # why: new workflow; batch03"].sort(), legacy())).not.toThrow();
});

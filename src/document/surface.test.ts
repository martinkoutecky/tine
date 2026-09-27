import { readFileSync } from "node:fs";
import ts from "typescript";
import { expect, it } from "vitest";

it("pins the document public surface", () => {
  const source = readFileSync("src/document/index.ts", "utf8");
  const listed = readFileSync("src/document/SURFACE.txt", "utf8").trim().split("\n");
  const file = ts.createSourceFile("index.ts", source, ts.ScriptTarget.Latest, true);
  const declarations = file.statements.filter(ts.isExportDeclaration);
  expect(declarations.every((statement) => statement.exportClause && ts.isNamedExports(statement.exportClause))).toBe(true);
  expect(file.statements.some((statement) => ts.isExportAssignment(statement) ||
    (!ts.isExportDeclaration(statement) && ts.canHaveModifiers(statement) &&
      ts.getModifiers(statement)?.some((modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword)))).toBe(false);
  const actual = file.statements.flatMap((statement) =>
    ts.isExportDeclaration(statement) && statement.exportClause && ts.isNamedExports(statement.exportClause)
      ? statement.exportClause.elements.map((element) => element.name.text)
      : []
  );
  expect(listed).toEqual([...new Set(listed)].sort());
  expect(actual.sort()).toEqual(listed);
});

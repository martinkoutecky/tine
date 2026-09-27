import { readFileSync } from "node:fs";
import ts from "typescript";
import { expect, it } from "vitest";

const MAX_EXPORTS = 135;

function checkSurface(source: string, listed: string[]): void {
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
  if (listed.length > MAX_EXPORTS || actual.length > MAX_EXPORTS || actual.sort().join("\n") !== listed.join("\n"))
    throw new Error("I-11: src/document/index.ts must match SURFACE.txt and may not grow; exemplar src/document/index.ts");
}

it("pins the document public surface", () => {
  checkSurface(readFileSync("src/document/index.ts", "utf8"),
    readFileSync("src/document/SURFACE.txt", "utf8").trim().split("\n"));
});

it("rejects a planted surface addition", () => {
  const source = readFileSync("src/document/index.ts", "utf8") + '\nexport { planted } from "./model";\n';
  const listed = readFileSync("src/document/SURFACE.txt", "utf8").trim().split("\n");
  expect(() => checkSurface(source, [...listed, "planted"].sort())).toThrow("I-11: src/document/index.ts must match SURFACE.txt and may not grow");
});

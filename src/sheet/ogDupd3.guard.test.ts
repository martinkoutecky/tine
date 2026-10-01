import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import ts from "typescript";

it("I-12: sheet scalar/calendar policy lives in typed.ts; imitate formula/value.ts", () => {
  for (const path of ["src/sheet/aggregate.ts", "src/sheet/formula/value.ts", "src/sheet/formula/eval.ts"]) {
    const source = readFileSync(path, "utf8");
    expect(source, `I-12: ${path} must use typed.ts for sheet number/calendar policy`).not.toMatch(/parseFloat\(|Date\.UTC\(|function daysInMonth\(/);
    expect(source).toMatch(/from ["'](?:\.\/|\.\.\/)typed["']/);
  }
  const typed = readFileSync("src/sheet/typed.ts", "utf8");
  expect(typed, "I-12: typed.ts validates calendar parts without allocating a Date per cell").not.toContain("new Date(");
});

function tokenGrammars(source: string): string[] {
  const out: string[] = [];
  const visit = (node: ts.Node) => {
    if (node.kind === ts.SyntaxKind.RegularExpressionLiteral) {
      const regex = node.getText();
      if (regex.includes("count|sum|avg") || regex.includes("[A-Za-z-]+")
        || regex.includes("[a-z-]+") || regex.includes("[=;\\0\\r\\n]")
        || regex.includes("[;=\\0\\r\\n]")) out.push(regex);
    }
    ts.forEachChild(node, visit);
  };
  visit(ts.createSourceFile("probe.ts", source, ts.ScriptTarget.Latest, true));
  return out;
}

it("I-12: aggregate segments use aggregate.ts and query field tokens use tablePresentation.ts", () => {
  for (const path of ["src/editor/queryViewProperties.ts", "src/editor/queryDisplayDraft.ts",
    "src/components/QueryVocabularyPicker.tsx", "src/sheet/queryTableFooter.ts", "src/sheet/renameField.ts"]) {
    expect(tokenGrammars(readFileSync(path, "utf8")),
      `I-12: ${path} must use decodeAggregateSegment in aggregate.ts or queryFieldEncodable in tablePresentation.ts`).toEqual([]);
  }
  const config = readFileSync("src/sheet/config.ts", "utf8");
  expect(tokenGrammars(config), "I-12: config.ts reads aggregates through aggregate.ts").toEqual([]);
  const rename = readFileSync("src/sheet/renameField.ts", "utf8");
  expect(rename, "I-12: schema key spans are owned by config.ts visitFieldSchema").not.toContain('indexOf("=")');
  expect(rename).toContain("visitFieldSchema(");
});

it("I-12: Hiccup's frozen EDN scalar subset has one numeric recognizer in hiccup.ts", () => {
  const source = readFileSync("src/render/hiccup.ts", "utf8");
  expect(source.match(/\^\[-\+\]\?/g), "I-12: share HICCUP_NUMBER between lookahead and consumption in hiccup.ts").toHaveLength(1);
});

it("detects planted aggregate and stored-field twin grammars", () => {
  expect(tokenGrammars('const x = /^(cost)=(count|sum|avg)$/;')).toHaveLength(1);
  expect(tokenGrammars('const x = /[=;\\0\\r\\n]/;')).toHaveLength(1);
});

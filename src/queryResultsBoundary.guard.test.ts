import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import ts from "typescript";

const rule = "I-12: resolve result locators through queryResultIdentity.ts::resolveResultRow; keep positional lookup at that boundary";
const consumers = ["components/QueryCollapse.ts", "components/queryResultsCopy.ts", "components/SourceTaskCheckbox.tsx",
  "components/LiveRefGroup.tsx", "sheet/formulaEval.ts", "components/QueryResultTask.tsx", "components/SheetTable.tsx", "components/SheetBoard.tsx"];

it("I-12: result consumers use the shared boundary and never directly look up a result's id", () => {
  for (const file of consumers) {
    const code = readFileSync(`src/${file}`, "utf8");
    const source = ts.createSourceFile(file, code, ts.ScriptTarget.Latest, true, file.endsWith("tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const bad: string[] = [];
    const visit = (node: ts.Node) => {
      if (ts.isCallExpression(node) && ["node", "docNode", "resolveBlockRef"].includes(node.expression.getText(source))) {
        const argument = node.arguments[0]?.getText(source) ?? "";
        if (/\b(?:row|block|hit|dto)(?:\b|\.)/.test(argument) && /\.id\b/.test(argument)) bad.push(node.getText(source));
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
    expect(bad, `${rule}: ${file}`).toEqual([]);
    expect(code, `${rule}: ${file}`).toMatch(/resolveResultRow|liveFormulaRowNode|SourceTaskCheckbox/);
  }
  const boundary = readFileSync("src/components/queryResultIdentity.ts", "utf8");
  expect(boundary, rule).toContain("resultsTicketActionable");
  expect(boundary, rule).not.toMatch(/contentFingerprint|sameQueryResult/);
});

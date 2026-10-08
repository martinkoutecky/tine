import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const read = (path: string) => readFileSync(path, "utf8");
it("I-12: visible preorder belongs to document/tree.ts visitVisible; find traversal to inpageFind.ts appendOutlineMatches", () => {
  const tree = read("src/document/tree.ts");
  expect(tree.match(/function visitVisible\(/g)).toHaveLength(1);
  expect(tree).not.toContain("const walk =");
  expect(tree.match(/visitVisible\((?:p|page|scope)\.roots/g)).toHaveLength(3);
  const find = read("src/inpageFind.ts");
  expect(find).not.toContain("const walk =");
  expect(find).toContain("appendOutlineMatches(blocks, dtoFindNode");
  expect(find).toContain("appendOutlineMatches(scope.roots, docNode");
});
it("I-12: imported outline construction is blocks.ts createOutline; deletion is subtree.ts removeSubtree; page mutation is moves.ts reassignPage", () => {
  const blocks = read("src/document/edits/blocks.ts");
  expect(blocks.match(/const create =/g)).toHaveLength(1);
  expect(blocks.match(/createOutline\((?:s|state), nodes/g)).toHaveLength(3);
  for (const name of ["blocks", "selection"]) {
    const source = read(`src/document/edits/${name}.ts`);
    expect(source).toContain("removeSubtree(s, id)");
    expect(source).not.toContain("const rm =");
  }
  const moves = read("src/document/edits/moves.ts");
  expect(moves).not.toContain("const reassign =");
  expect(moves.match(/\.page\s*=(?!=)/g)).toHaveLength(1);
});
it("I-12: external opening, month stepping and option publication use components/primitives.ts; raw formula commits use FormulaEditor.tsx createAstCommit", () => {
  for (const name of ["AboutTab", "HelpShortcuts"]) {
    const source = read(`src/components/${name}.tsx`);
    expect(source).toContain('from "./primitives"');
    expect(source).not.toContain(".openExternal(");
  }
  for (const name of ["CalendarJump", "DatePicker"]) {
    const source = read(`src/components/${name}.tsx`);
    expect(source).toContain("stepMonth(view(), delta)");
    expect(source).not.toContain("% 12");
  }
  for (const name of ["ExportModal", "PdfExportDialog"]) {
    const source = read(`src/components/${name}.tsx`);
    expect(source).toContain("optionsUpdater(opts, setOpts,");
    expect(source).not.toContain("...patch");
  }
  const formula = read("src/components/FormulaEditor.tsx");
  expect(formula.match(/parseAstText\(value\(\)\)/g)).toHaveLength(1);
  expect(formula.match(/= createAstCommit\(/g)).toHaveLength(2);
  const check = read("crates/tine-store/src/bin/tine-check.rs");
  expect(check).toContain("use tine_core::org::trailing_newlines;");
  expect(check).not.toContain("fn trailing_newlines(");
});
it("I-12: board field and formula reference codecs belong to sheet/boardColumns.ts", () => {
  for (const name of ["ContextMenu", "SheetBoard", "SheetTable"]) {
    const source = read(`src/components/${name}.tsx`);
    expect(source).toContain("formulaReferenceName");
    expect(source).not.toContain("function formulaReferenceName(");
  }
  const menu = read("src/components/ContextMenu.tsx");
  expect(menu).toContain("groupFieldForToken(props.groupBy)");
  expect(menu).not.toContain('raw.startsWith("formula.")');
});
it("I-12: body and standalone macro viewport lifecycle use createNearBlockMount.ts", () => {
  for (const path of ["src/render/body.tsx", "src/components/DeferredStandaloneMacro.tsx"]) {
    const source = read(path);
    expect(source).toContain("createNearBlockMount(props");
    expect(source).not.toMatch(/observeNear\(|unobserveNear\(|renderedBlocks\.(?:add|has)/);
  }
});

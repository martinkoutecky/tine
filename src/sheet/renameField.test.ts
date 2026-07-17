import { describe, expect, it } from "vitest";
import { astToExpr, parseFormula } from "./formula";
import {
  planSheetFieldRename,
  propertyOccurrences,
  renameCanonicalPropertyKey,
  rewriteAggregateValue,
  rewriteFieldAst,
  rewriteSchemaValueLosslessly,
  type RenameSource,
} from "./renameField";
import type { Format } from "../render/ast";

function source(id: string, raw: string, format: Format = "md", page = "Sheet"): RenameSource {
  return {
    id,
    page,
    raw,
    format,
    recognizedProperties: propertyOccurrences(raw, format).map((item) => [item.key, item.value]),
  };
}

describe("Sheet field rename planner", () => {
  it("renames canonical Markdown and Org keys in place without touching body or fences", () => {
    const md = [
      "Row",
      "SCHEDULED: <2026-07-16 Thu>",
      "Occurrence:: 2",
      "body",
      "```",
      "Occurrence:: in-code",
      "```",
      "tail:: kept",
    ].join("\n");
    expect(renameCanonicalPropertyKey(md, "md", "Occurrence", "OCC")).toEqual({
      ok: true,
      count: 1,
      raw: md.replace("Occurrence:: 2", "OCC:: 2"),
    });

    const org = [
      "Row",
      "SCHEDULED: <2026-07-16 Thu>",
      ":PROPERTIES:",
      "  :Occurrence:   2",
      "  :other: kept",
      ":END:",
      "Occurrence:: body",
    ].join("\n");
    expect(renameCanonicalPropertyKey(org, "org", "Occurrence", "OCC")).toEqual({
      ok: true,
      count: 1,
      raw: org.replace(":Occurrence:", ":OCC:"),
    });
  });

  it("rewrites only exact field AST nodes through nested unary/binary/call/member expressions", () => {
    const parsed = parseFormula('!occurrence || fn(occurrence, "occurrence", formula.occurrence).method(occurrence)');
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const rewritten = rewriteFieldAst(parsed.ast, "occurrence", "OCC");
    expect(rewritten.changed).toBe(true);
    expect(astToExpr(rewritten.ast)).toBe('!OCC || fn(OCC, "occurrence", formula.occurrence).method(OCC)');
  });

  it("rewrites one schema identity while preserving malformed and unrelated segments byte-for-byte", () => {
    expect(rewriteSchemaValueLosslessly(" severity=number ; bad segment ; occurrence = number ;x=enum:a,b", "occurrence", "OCC"))
      .toEqual({ ok: true, value: " severity=number ; bad segment ; OCC = number ;x=enum:a,b" });
  });

  it("rewrites exact group aggregate identities and rejects malformed or ambiguous aggregate config", () => {
    expect(rewriteAggregateValue("prop:severity=max; prop:occurrence = sum ;formula:occurrence=count", "occurrence", "OCC"))
      .toEqual({ ok: true, value: "prop:severity=max; prop:OCC = sum ;formula:occurrence=count" });
    expect(rewriteAggregateValue("prop:occurrence=sum;broken", "occurrence", "OCC")).toMatchObject({ ok: false });
    expect(rewriteAggregateValue("prop:x=sum;PROP:X=max", "occurrence", "OCC")).toMatchObject({ ok: false });
  });

  it("builds one complete owner/row migration while preserving literals, formula refs, and malformed schema segments", () => {
    const owner = source("table", [
      "Table",
      "TINE.FIELDS:: severity=number; malformed ;occurrence=number;detection=number",
      'tine.formula.rpn:: severity * occurrence * detection + if(label == "occurrence", formula.occurrence, 0)',
      "tine.filter:: occurrence > 1",
      "tine.group-by:: prop:occurrence",
      "tine.col-aggregates:: prop:occurrence=sum;prop:severity=max",
    ].join("\n"));
    const r1 = source("r1", "Row one\noccurrence:: 2\nseverity:: 3\ndetection:: 4");
    const result = planSheetFieldRename({
      rowSource: "children",
      ownerWritable: true,
      schemaHome: "block",
      owner,
      rows: [r1],
      oldField: "prop:occurrence",
      newName: "OCC",
    });
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.plan.ownerRaw).toContain("TINE.FIELDS:: severity=number; malformed ;OCC=number;detection=number");
    expect(result.plan.ownerRaw).toContain('tine.formula.rpn:: severity * OCC * detection + if(label == "occurrence", formula.occurrence, 0)');
    expect(result.plan.ownerRaw).toContain("tine.filter:: OCC > 1");
    expect(result.plan.ownerRaw).toContain("tine.group-by:: prop:OCC");
    expect(result.plan.ownerRaw).toContain("tine.col-aggregates:: prop:OCC=sum;prop:severity=max");
    expect(result.plan.rows).toEqual([{ id: "r1", raw: "Row one\nOCC:: 2\nseverity:: 3\ndetection:: 4" }]);
  });

  it.each([
    ["query-backed", { rowSource: "query" as const }, "children-backed"],
    ["read-only", { ownerWritable: false }, "read-only"],
    ["inherited", { schemaHome: "page" as const }, "block-local"],
    ["invalid name", { newName: "not addressable" }, "Field names"],
    ["formula literal", { newName: "true" }, "reserved"],
  ])("rejects %s preflight without producing candidates", (_label, override, message) => {
    const base = {
      rowSource: "children" as const,
      ownerWritable: true,
      schemaHome: "block" as const,
      owner: source("table", "Table\ntine.fields:: occurrence=number"),
      rows: [source("r1", "Row\noccurrence:: 2")],
      oldField: "prop:occurrence",
      newName: "OCC",
    };
    const result = planSheetFieldRename({ ...base, ...override });
    expect(result).toMatchObject({ ok: false });
    if (!result.ok) expect(result.error).toContain(message);
  });

  it.each([
    ["builtin", "state", "built-in"],
    ["declared", "severity", "declared"],
    ["observed", "note", "already has"],
    ["case-insensitive row", "NOTE", "already has"],
  ])("rejects a %s collision", (_label, newName, message) => {
    const result = planSheetFieldRename({
      rowSource: "children",
      ownerWritable: true,
      schemaHome: "block",
      owner: source("table", "Table\ntine.fields:: severity=number;occurrence=number"),
      rows: [source("r1", "Row\noccurrence:: 2\nnote:: kept")],
      oldField: "prop:occurrence",
      newName,
    });
    expect(result).toMatchObject({ ok: false });
    if (!result.ok) expect(result.error).toContain(message);
  });

  it("supports a case-only rename and preserves config-key spelling", () => {
    const result = planSheetFieldRename({
      rowSource: "children",
      ownerWritable: true,
      schemaHome: "block",
      owner: source("table", "Table\nTiNe.FiElDs:: occurrence=number\nTiNe.FoRmUlA.rpn:: occurrence * 2"),
      rows: [source("r1", "Row\noccurrence:: 2")],
      oldField: "prop:occurrence",
      newName: "Occurrence",
    });
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.plan.ownerRaw).toContain("TiNe.FiElDs:: Occurrence=number");
    expect(result.plan.ownerRaw).toContain("TiNe.FoRmUlA.rpn:: Occurrence * 2");
    expect(result.plan.rows[0].raw).toContain("Occurrence:: 2");
  });

  it("rejects duplicate case-insensitive config keys, local formula variants, and duplicate row keys", () => {
    const common = {
      rowSource: "children" as const,
      ownerWritable: true,
      schemaHome: "block" as const,
      oldField: "prop:occurrence",
      newName: "OCC",
    };
    expect(planSheetFieldRename({
      ...common,
      owner: source("table", "Table\ntine.fields:: occurrence=number\nTINE.FIELDS:: occurrence=number"),
      rows: [],
    })).toMatchObject({ ok: false });
    expect(planSheetFieldRename({
      ...common,
      owner: source("table", "Table\ntine.fields:: occurrence=number\ntine.formula.rpn:: occurrence\nTINE.FORMULA.rpn:: occurrence"),
      rows: [],
    })).toMatchObject({ ok: false });
    expect(planSheetFieldRename({
      ...common,
      owner: source("table", "Table\ntine.fields:: occurrence=number"),
      rows: [source("r1", "Row\noccurrence:: 1\noccurrence:: 2")],
    })).toMatchObject({ ok: false });
  });

  it("honors formula shadowing and aborts only for an affected effective page formula", () => {
    const base = {
      rowSource: "children" as const,
      ownerWritable: true,
      schemaHome: "block" as const,
      rows: [] as RenameSource[],
      oldField: "prop:occurrence",
      newName: "OCC",
    };
    const shadowed = planSheetFieldRename({
      ...base,
      owner: source("table", "Table\ntine.fields:: occurrence=number\ntine.formula.rpn:: occurrence * 2"),
      pageProperties: [["tine.formula.rpn", "occurrence * 99"]],
    });
    expect(shadowed.ok).toBe(true);

    const pageOwned = planSheetFieldRename({
      ...base,
      owner: source("table", "Table\ntine.fields:: occurrence=number"),
      pageProperties: [["tine.formula.rpn", "occurrence * 99"]],
    });
    expect(pageOwned).toMatchObject({ ok: false });
    if (!pageOwned.ok) expect(pageOwned.error).toContain("page-owned");
  });

  it("aborts on malformed formulas and filters before yielding any write plan", () => {
    for (const config of ["tine.formula.rpn:: occurrence +", "tine.filter:: occurrence >"]) {
      const result = planSheetFieldRename({
        rowSource: "children",
        ownerWritable: true,
        schemaHome: "block",
        owner: source("table", `Table\ntine.fields:: occurrence=number\n${config}`),
        rows: [source("r1", "Row\noccurrence:: 2")],
        oldField: "prop:occurrence",
        newName: "OCC",
      });
      expect(result).toMatchObject({ ok: false });
    }
  });
});

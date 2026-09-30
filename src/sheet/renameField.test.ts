import { sheetConfig } from "./config";
import { AGGREGATE_FNS, isAggregateFn } from "./aggregate";
import { initParser } from "../render/parse";
import { facetsOf } from "../render/facets";
beforeAll(initParser);
import { beforeAll, describe, expect, it } from "vitest";
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
  it("renames a field alongside canonical Unicode properties", () => {
    const owner = source("table", "Table\ntine.fields:: qty=number");
    const row = source("row", "Row\nqty:: 2\ncafé:: note");
    row.recognizedProperties = facetsOf(row.raw, "md").properties;
    const result = planSheetFieldRename({ rowSource: "children", ownerWritable: true, schemaHome: "block",
      owner, rows: [row], oldField: "prop:qty", newName: "amount" });
    expect(result.ok).toBe(true);
  });

  it("keeps Unicode Org properties and multibyte CRLF source offsets lossless", () => {
    const raw = "Žluťoučký\r\n:PROPERTIES:\r\n  :qty: 2\r\n  :café: note\r\n:END:";
    expect(renameCanonicalPropertyKey(raw, "org", "qty", "amount")).toEqual({
      ok: true, count: 1, raw: raw.replace(":qty:", ":amount:") });
    const md = "Žluťoučký\r\nqty:: 2\r\ncafé:: note";
    expect(renameCanonicalPropertyKey(md, "md", "qty", "amount")).toEqual({
      ok: true, count: 1, raw: md.replace("qty::", "amount::") });
  });

  it("round-trips an Org owner and rows with canonical facet verification", () => {
    const owner = source("table", "Table\n:PROPERTIES:\n:tine.view: table\n:tine.fields: occurrence=number\n:tine.formula.rpn: occurrence * 2\n:END:\nbody", "org");
    const row = source("row", "Row\n:PROPERTIES:\n:occurrence: 2\n:other: kept\n:END:\nbody", "org");
    owner.recognizedProperties = facetsOf(owner.raw, "org").properties;
    row.recognizedProperties = facetsOf(row.raw, "org").properties;
    expect(propertyOccurrences(owner.raw, "org").map((p) => [p.key, p.value])).toEqual(owner.recognizedProperties);
    const result = planSheetFieldRename({ rowSource: "children", ownerWritable: true, schemaHome: "block",
      owner, rows: [row], recognizeProperties: (raw, format) => facetsOf(raw, format).properties,
      oldField: "prop:occurrence", newName: "OCC" });
    expect(result, JSON.stringify(result)).toMatchObject({ ok: true });
  });

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

  it("rewrites exact group aggregate identities and leaves every other segment where it was", () => {
    expect(rewriteAggregateValue("prop:severity=max; prop:occurrence = sum ;formula:occurrence=count", "occurrence", "OCC"))
      .toEqual({ ok: true, value: "prop:severity=max; prop:OCC = sum ;formula:occurrence=count" });
    // An unrecognized segment that has nothing to do with this rename is
    // preserved verbatim rather than failing the whole rename (P5A). Refusing
    // it made a query configuration un-renameable for no gain.
    expect(rewriteAggregateValue("prop:occurrence=sum;broken", "occurrence", "OCC"))
      .toEqual({ ok: true, value: "prop:OCC=sum;broken" });
    // Two keys that collide with each other but not with the renamed field are
    // somebody else's problem: this rename touches neither.
    expect(rewriteAggregateValue("prop:x=sum;PROP:X=max", "occurrence", "OCC"))
      .toEqual({ ok: true, value: "prop:x=sum;PROP:X=max" });
  });

  /** **The preservation debt** (P5A). `tine.col-aggregates` is shared ground:
   *  the query reader spells the whole-result count as a bare `count`, allows
   *  `avg`, and treats its entries as an ordered list with repeats; the sheet
   *  footer has its own seventeen-name vocabulary. The rename helper has to
   *  cross that boundary without either refusing ordinary query values or
   *  claiming the sheet can execute a function it has no implementation for. */
  it("carries a query aggregate configuration across a sheet field rename", () => {
    // A bare `count` is the whole-result count (X3), not a malformed segment;
    // and `cost` is a BARE query key, outside the sheet's `prop:` rename
    // ownership, so renaming the sheet field `cost` must not touch it.
    expect(rewriteAggregateValue("count;cost=sum", "cost", "price"))
      .toEqual({ ok: true, value: "count;cost=sum" });
    // Repeated keys are retained, in order, and both are renamed. `avg` is
    // recognized without becoming a sheet AggregateFn.
    expect(rewriteAggregateValue("count;prop:cost=sum;prop:cost=avg", "cost", "price"))
      .toEqual({ ok: true, value: "count;prop:price=sum;prop:price=avg" });
    // Unknown unrelated segments survive untouched, wherever they sit.
    expect(rewriteAggregateValue("weird stuff;prop:cost=avg;also weird", "cost", "price"))
      .toEqual({ ok: true, value: "weird stuff;prop:price=avg;also weird" });
    // The refusals that remain: an unparseable segment that names the field
    // being renamed, and a key that differs only by case.
    expect(rewriteAggregateValue("prop:cost=sum;prop:cost", "cost", "price"))
      .toMatchObject({ ok: false });
    expect(rewriteAggregateValue("prop:Cost=sum", "cost", "price")).toMatchObject({ ok: false });
  });

  it("does not widen the sheet aggregate vocabulary to make avg renameable", () => {
    // N6(i): `cost=avg` must not become a "valid" sheet aggregate — it has no
    // arm in `applyAggregate` and no entry in the publisher's function list.
    expect(isAggregateFn("avg")).toBe(false);
    expect(AGGREGATE_FNS).not.toContain("avg");
    expect(sheetConfig([["tine.col-aggregates", "prop:cost=avg"]]).colAggregates.size).toBe(0);
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

it.each([40, 6000])("field rename handles a %i-term imported formula without throwing", (terms) => {
  const owner = source("table", `Table\ntine.fields:: qty=number\ntine.formula.total:: ${Array(terms).fill("qty").join(" + ")}`);
  const result = planSheetFieldRename({ rowSource: "children", ownerWritable: true, schemaHome: "block", owner, rows: [source("row", "Row\nqty:: 2")], oldField: "prop:qty", newName: "amount" });
  if (terms === 40) expect(result.ok).toBe(true);
  else expect(result).toMatchObject({ ok: false, error: expect.stringContaining("depth") });
});

it("I-12: planSheetFieldRename preserves ordered query aggregates through the shared lossless answerer", () => {
  const result = planSheetFieldRename({ rowSource: "children", ownerWritable: true, schemaHome: "block",
    owner: source("owner", "Table\ntine.fields:: cost=number\ntine.col-aggregates:: count;prop:cost=sum;prop:cost=avg;unrelated"),
    rows: [source("row", "Row\ncost:: 2")], oldField: "prop:cost", newName: "price" });
  expect(result.ok).toBe(true);
  if (result.ok) expect(result.plan.ownerRaw).toContain("tine.col-aggregates:: count;prop:price=sum;prop:price=avg;unrelated");
});

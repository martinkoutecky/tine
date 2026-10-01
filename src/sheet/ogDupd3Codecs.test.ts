import { expect, it } from "vitest";
import { decodeAggregateSegment } from "./aggregate";
import { parseFields, sheetConfig } from "./config";
import { rewriteAggregateValue, rewriteSchemaValueLosslessly } from "./renameField";
import { mergeQueryAggregateValue } from "../editor/queryViewProperties";
import { normalizeQueryDisplayDraft } from "../editor/queryDisplayDraft";
import { queryColumnName, querySortFieldName } from "./tablePresentation";
import { parseFormula } from "./formula/parser";
import { parseDurationValue } from "./formula/value";
import { hiccupToHtml } from "../render/hiccup";
import { parseDelimitedText } from "./tsv";

it("D35: aggregate codecs retain their named sheet, query and rename policies", () => {
  const raw = "count; prop:cost=sum ;prop:cost=avg;prop:cost=median;opaque";
  expect([...sheetConfig([["tine.col-aggregates", raw]]).colAggregates]).toEqual([["prop:cost", "median"]]);
  expect(mergeQueryAggregateValue(raw, [["", "count"], ["prop:cost", "sum"], ["prop:cost", "avg"]])).toBeUndefined();
  expect(rewriteAggregateValue(raw, "cost", "price")).toEqual({ ok: true,
    value: "count; prop:price=sum ;prop:price=avg;prop:price=median;opaque" });
  expect(decodeAggregateSegment("cost=SUM", "sheet")).toBeNull();
  expect(decodeAggregateSegment("cost=SUM", "query")).not.toBeNull();
  expect(decodeAggregateSegment("cost= sum", "query")).toBeNull();
  expect(decodeAggregateSegment("cost= sum", "sheet")).not.toBeNull();
});

it("D35: schema rename keeps unknown tokens and whitespace; type admission remains separate", () => {
  const raw = " cost = number ; other = future=type ; state=state";
  expect(parseFields(raw)).toEqual([{ field: "prop:cost", type: "number" }, { field: "state", type: "builtin" }]);
  expect(rewriteSchemaValueLosslessly(raw, "cost", "price")).toEqual({ ok: true,
    value: " price = number ; other = future=type ; state=state" });
  expect(rewriteSchemaValueLosslessly("cost=number;COST=unknown", "cost", "price").ok).toBe(false);
});

it("D35: stored query field tokens share punctuation rules with distinct draft/builtin bounds", () => {
  for (const name of ["cost", "成本", "with space"]) {
    expect(queryColumnName(`prop:${name}`)).toBe(name);
    expect(querySortFieldName(`prop:${name}`)).toBe(name);
    expect(normalizeQueryDisplayDraft({ columns: [name] })).toEqual({ columns: [name] });
  }
  for (const name of ["a=b", "a;b", "a\0b", "a\nb", "a\rb", ""]) {
    expect(queryColumnName(`prop:${name}`)).toBeNull();
    expect(querySortFieldName(`prop:${name}`)).toBeNull();
    expect(normalizeQueryDisplayDraft({ columns: [name] })).toBeNull();
  }
  expect(queryColumnName("prop:page")).toBeNull();
  expect(querySortFieldName("prop:page")).toBe("page");
  expect(queryColumnName("prop: padded ")).toBe(" padded ");
  expect(normalizeQueryDisplayDraft({ columns: [" padded "] })).toBeNull();
});

it("D36: expression, Hiccup EDN, duration and CSV are distinct scalar languages", () => {
  for (const token of [".5", "1.", "1e2"]) {
    expect(parseFormula(token).ok).toBe(false);
    expect(hiccupToHtml(`[:span ${token}]`)).toBe(`<span>${token}</span>`);
  }
  expect(parseFormula("12.5").ok).toBe(true);
  expect(parseDurationValue("1M")).toEqual({ kind: "duration", n: 1, unit: "M" });
  expect(parseDurationValue("1m")).toEqual({ kind: "duration", n: 1, unit: "m" });
  expect(parseDurationValue("1.5h")).toBeNull();
  expect(parseDelimitedText('1e2,"0099-01-01",1M', 'csv')).toEqual([["1e2", "0099-01-01", "1M"]]);
});

import type { FieldValue } from "./fields";
import { isoDatePrefix, sheetNumber } from "./typed";
import type { QueryAggFn } from "../editor/queryAggregate";

export type AggregateFn =
  | "sum"
  | "average"
  | "median"
  | "min"
  | "max"
  | "range"
  | "stddev"
  | "earliest"
  | "latest"
  | "empty"
  | "filled"
  | "unique"
  | "checked"
  | "unchecked"
  | "count";

export const AGGREGATE_FNS: readonly AggregateFn[] = [
  "sum",
  "average",
  "median",
  "min",
  "max",
  "range",
  "stddev",
  "earliest",
  "latest",
  "empty",
  "filled",
  "unique",
  "checked",
  "unchecked",
  "count",
];

export const AGGREGATE_LABELS: Record<AggregateFn, string> = {
  sum: "Sum",
  average: "Average",
  median: "Median",
  min: "Min",
  max: "Max",
  range: "Range",
  stddev: "Stddev",
  earliest: "Earliest",
  latest: "Latest",
  empty: "Empty",
  filled: "Filled",
  unique: "Unique",
  checked: "Checked",
  unchecked: "Unchecked",
  count: "Count",
};

const AGGREGATE_SET = new Set<string>(AGGREGATE_FNS);

export function isAggregateFn(value: string): value is AggregateFn {
  return AGGREGATE_SET.has(value);
}

export interface AggregateSegment { readonly keyStart: number; readonly keyEnd: number; readonly fn: string }
const BARE_QUERY_COUNT: AggregateSegment = { keyStart: 0, keyEnd: 0, fn: "count" };
const SPACE = /\s/;
const QUERY_AGGREGATE_FNS: readonly string[] = ["count", "sum", "avg"];

export function isQueryAggregateFn(value: string): value is QueryAggFn {
  return QUERY_AGGREGATE_FNS.includes(value);
}

/** Decode one stored aggregate segment, O(segment bytes), retaining spelling
 * and key spans for lossless rename. Sheet maps accept lowercase sheet functions;
 * query lists accept count/sum/avg and bare count, preserving duplicates. Rename
 * owns the union (case-insensitive) and leaves unreadable segments to its caller.
 * These are codecs for property VALUES, never readers of Logseq structure. */
export function decodeAggregateSegment(segment: string, policy: "sheet" | "query" | "rename"): AggregateSegment | null {
  if (policy === "query" && segment.trim().toLowerCase() === "count")
    return BARE_QUERY_COUNT;
  const eq = segment.indexOf("=");
  if (eq < 0 || segment.indexOf("=", eq + 1) >= 0 || segment.includes(";")) return null;
  let keyStart = 0, keyEnd = eq, fnStart = eq + 1, fnEnd = segment.length;
  while (keyStart < keyEnd && SPACE.test(segment[keyStart])) keyStart++;
  while (keyEnd > keyStart && SPACE.test(segment[keyEnd - 1])) keyEnd--;
  while (fnStart < fnEnd && SPACE.test(segment[fnStart])) fnStart++;
  while (fnEnd > fnStart && SPACE.test(segment[fnEnd - 1])) fnEnd--;
  const fn = segment.slice(fnStart, fnEnd);
  const normalized = fn.toLowerCase();
  if (policy === "query") {
    // Rust `parse_col_aggregate_segment` trims both sides of '='; the shared
    // golden tests/fixtures/i12-col-aggregates-golden.json pins this policy.
    if (!isQueryAggregateFn(normalized)) return null;
  } else if (keyStart === keyEnd || (!isAggregateFn(normalized) && !(policy === "rename" && normalized === "avg"))
    || (policy === "sheet" && fn !== normalized)) return null;
  return { keyStart, keyEnd, fn };
}

/** Query aggregate spelling; ordered entries are never reduced to a map. */
export function encodeQueryAggregate([field, fn]: readonly [string, QueryAggFn]): string {
  return field ? `${field}=${fn}` : fn;
}

function textOf(value: FieldValue | string | null | undefined): string {
  if (value == null) return "";
  return typeof value === "string" ? value : value.raw ?? value.text;
}

function formatNumber(n: number): string {
  if (!Number.isFinite(n)) return "0";
  const rounded = Math.round(n * 1000) / 1000;
  return `${Object.is(rounded, -0) ? 0 : rounded}`;
}

function withSkipped(text: string, skipped: number): string {
  if (skipped <= 0) return text;
  return text ? `${text} (${skipped} skipped)` : `(${skipped} skipped)`;
}

function numericValues(values: readonly (FieldValue | string | null | undefined)[]): { nums: number[]; skipped: number } {
  const nums: number[] = [];
  let skipped = 0;
  for (const value of values) {
    const text = textOf(value).trim();
    const n = sheetNumber(text, "aggregate-prefix");
    if (n !== null) nums.push(n);
    else skipped++;
  }
  return { nums, skipped };
}

function dateValues(values: readonly (FieldValue | string | null | undefined)[]): { dates: string[]; skipped: number; numericNonDates: number } {
  const dates: string[] = [];
  let skipped = 0;
  let numericNonDates = 0;
  for (const value of values) {
    const text = textOf(value).trim();
    const iso = isoDatePrefix(text);
    if (iso) {
      dates.push(iso);
    } else {
      if (sheetNumber(text, "aggregate-prefix") !== null) numericNonDates++;
      skipped++;
    }
  }
  dates.sort();
  return { dates, skipped, numericNonDates };
}

function isCheckedText(text: string): boolean {
  const lower = text.trim().toLowerCase();
  return lower === "done" || lower === "true" || lower === "yes" || lower === "checked" || lower === "x" || lower === "[x]";
}

function aggregateNumbers(fn: AggregateFn, values: readonly (FieldValue | string | null | undefined)[]): string {
  const { nums, skipped } = numericValues(values);
  if (!nums.length) return withSkipped("0", skipped);
  nums.sort((a, b) => a - b);
  switch (fn) {
    case "sum":
      return withSkipped(formatNumber(nums.reduce((a, b) => a + b, 0)), skipped);
    case "average":
      return withSkipped(formatNumber(nums.reduce((a, b) => a + b, 0) / nums.length), skipped);
    case "median": {
      const mid = Math.floor(nums.length / 2);
      const val = nums.length % 2 ? nums[mid] : (nums[mid - 1] + nums[mid]) / 2;
      return withSkipped(formatNumber(val), skipped);
    }
    case "min":
      return withSkipped(formatNumber(nums[0]), skipped);
    case "max":
      return withSkipped(formatNumber(nums[nums.length - 1]), skipped);
    case "range":
      return withSkipped(formatNumber(nums[nums.length - 1] - nums[0]), skipped);
    case "stddev": {
      const mean = nums.reduce((a, b) => a + b, 0) / nums.length;
      const variance = nums.reduce((sum, n) => sum + (n - mean) ** 2, 0) / nums.length;
      return withSkipped(formatNumber(Math.sqrt(variance)), skipped);
    }
    default:
      return withSkipped("0", skipped);
  }
}

function aggregateDates(fn: AggregateFn, values: readonly (FieldValue | string | null | undefined)[]): string {
  const { dates, skipped } = dateValues(values);
  if (!dates.length) return withSkipped("", skipped);
  if (fn === "earliest") return withSkipped(dates[0], skipped);
  if (fn === "latest") return withSkipped(dates[dates.length - 1], skipped);
  return withSkipped(`${dates[0]} - ${dates[dates.length - 1]}`, skipped);
}

export function aggregate(fn: AggregateFn, values: readonly (FieldValue | string | null | undefined)[]): string {
  if (fn === "empty") return `${values.filter((v) => textOf(v).trim() === "").length}`;
  if (fn === "filled" || fn === "count") return `${values.filter((v) => textOf(v).trim() !== "").length}`;
  if (fn === "unique") return `${new Set(values.map((v) => textOf(v).trim()).filter(Boolean)).size}`;
  if (fn === "checked") return `${values.filter((v) => isCheckedText(textOf(v))).length}`;
  if (fn === "unchecked") return `${values.filter((v) => !isCheckedText(textOf(v))).length}`;
  if (fn === "earliest" || fn === "latest") return aggregateDates(fn, values);
  if (fn === "range") {
    const dates = dateValues(values);
    if (dates.dates.length > 0 && dates.numericNonDates === 0) return aggregateDates(fn, values);
  }
  return aggregateNumbers(fn, values);
}

export function collectAggregateColumns(
  rows: Iterable<{ cellIds: readonly string[] }>,
  columns: readonly number[],
  valueOf: (id: string | null) => string,
): ReadonlyMap<number, readonly string[]> {
  const result = new Map<number, string[]>();
  for (const column of columns) result.set(column, []);
  for (const row of rows) {
    for (const column of columns) {
      result.get(column)!.push(valueOf(row.cellIds[column] ?? null));
    }
  }
  return result;
}

import type { FieldValue } from "./fields";
import { isoDatePrefix } from "./typed";

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
    const n = parseFloat(text);
    if (Number.isFinite(n)) nums.push(n);
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
      if (Number.isFinite(parseFloat(text))) numericNonDates++;
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

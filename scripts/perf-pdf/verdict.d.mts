export type Family = { label: string; unit: string; floor: number; rel: number };
export const FAMILIES: Record<string, Family>;
export type Status = "regression" | "improvement" | "watch" | "same" | "n/a";
export type Comparison = {
  family: string; base: number[]; cand: number[]; baseMed: number | null; candMed: number | null;
  status: Status; gap: number | null; threshold: number | null; note?: string;
};
export type Result = Comparison & { dpr: number; cell: string };
export function median(xs: Array<number | null | undefined>): number | null;
export function sideOf(tag: string): "base" | "cand" | null;
export function compare(base: number[], cand: number[], family: string, baseMissing?: number, candMissing?: number): Comparison;
export function verdict(rows: Array<Record<string, unknown>>): {
  results: Result[]; regressions: Result[]; improvements: Result[]; watch: Result[];
};
export function resultTable(results: Result[], options?: { withBase?: boolean }): string;

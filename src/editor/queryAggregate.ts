// Query summaries format backend statistics from the same snapshot as the rows.

/** The three functions a QUERY aggregate can name (contract §4). The sheet's
 *  own seventeen-name footer vocabulary is a different, wider set, and is
 *  deliberately not reachable from here. */
export type QueryAggFn = "count" | "sum" | "avg";

/** One `tine.col-aggregates` entry as the QUERY reads it (contract §4): a bare
 *  `count` has an empty field and means the whole-result count. */
export type QueryAggregateEntry = readonly [field: string, fn: QueryAggFn];

export interface QuerySummaryCell {
  text: string;
  /** Rows that could not contribute — sum/avg over an absent or non-numeric
   *  value. Always 0 for a count. */
  skipped: number;
}

export interface QuerySummaryGroup {
  /** The group's own key; `null` for the rows that carry no value. */
  key: string | null;
  label: string;
  count: number;
  /** One cell per requested aggregate, in the requested order. */
  cells: QuerySummaryCell[];
}

export interface QuerySummary {
  notice?: string | null;
  /** One column per REQUESTED aggregate, in the requested order — repeats and a
   *  bare whole-result count included, because the view carries a LIST. */
  columns: { label: string; entry: QueryAggregateEntry }[];
  overall: QuerySummaryCell[];
  /** `null` when the result is not grouped. */
  groups: QuerySummaryGroup[] | null;
  /** The grouping field's label, for the breakdown's first column head. */
  groupLabel: string | null;
  /** Whether one row can sit in SEVERAL groups (tags), so the surface can say
   *  so rather than imply the groups partition the result. */
  multiMembership: boolean;
}

/** The user-facing name of one aggregate column. */
export function queryAggregateLabel([field, fn]: QueryAggregateEntry): string {
  const verb = fn === "count" ? "Count" : fn === "sum" ? "Sum" : "Avg";
  return field ? `${verb} of ${field}` : verb;
}

/** Format the snapshot's complete statistics. Rows and live editor facets never
 * participate in this adapter. An absent answer is not a numeric zero. */
export function querySummary(input: {
  statistics?: import("./queryIr").QueryStatistics | null;
  groupLabel?: string | null;
}): QuerySummary | null {
  const statistics = input.statistics;
  if (!statistics) return null;
  const format = (cell: import("./queryIr").QueryStatisticsCell): QuerySummaryCell => {
    if (cell.kind === "marker") return {
      text: `Unavailable (${cell.reason.replaceAll("_", " ")})`, skipped: cell.skipped,
    };
    // Avoid overflowing a finite large number while rounding for display.
    const scaled = cell.value * 1000;
    const value = Number.isFinite(scaled) ? Math.round(scaled) / 1000 : cell.value;
    return { text: `${value}`, skipped: cell.skipped };
  };
  return {
    columns: statistics.aggregates.map((entry) => ({ label: queryAggregateLabel(entry), entry })),
    overall: statistics.overall.map(format),
    groups: statistics.groups?.map((group) => ({
      key: group.key, label: group.key ?? "(none)", count: group.count, cells: group.cells.map(format),
    })) ?? null,
    groupLabel: input.groupLabel ?? statistics.group_by,
    multiMembership: statistics.group_by === "tags",
    notice: statistics.grouping_status === "unsupported_formula"
      ? "Exact statistics by formula are not supported yet. Overall statistics are shown."
      : null,
  };
}

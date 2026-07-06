// Pure result-aggregation helpers for {{query}} summaries (1a). The directives
// (`(aggregate …)` / `(group-by …)`) ride in the DSL and are parse-but-ignored by
// the Rust engine, which returns the full block set; the math is computed here in
// the frontend from the returned rows. Kept DOM-free + unit-testable.

export interface AggDirective {
  agg: "count" | "sum" | "avg";
  field: string | null;
}

// The minimal row shape the aggregation needs: a page (group key) and its parsed
// properties. Matches the `Row` the query renderer flattens from the block DTOs.
export interface AggRow {
  page: string;
  props: Record<string, string>;
}

export interface AggResult {
  text: string; // the aggregate value, formatted
  skipped: number; // rows that couldn't contribute (sum/avg: absent/non-numeric)
}

/** Fold a row set to the active aggregate. `null`/count → the row count. Sum/avg
 *  parse the chosen property with parseFloat (so "3 hrs" contributes 3); rows whose
 *  value is absent or non-numeric are counted as `skipped`. */
export function foldAggregate(set: AggRow[], agg: AggDirective | null): AggResult {
  if (!agg || agg.agg === "count") return { text: `${set.length}`, skipped: 0 };
  const field = agg.field ?? "";
  let sum = 0;
  let n = 0;
  let skipped = 0;
  for (const r of set) {
    const v = parseFloat((r.props[field] ?? "").trim());
    if (Number.isFinite(v)) {
      sum += v;
      n++;
    } else skipped++;
  }
  const val = agg.agg === "sum" ? sum : n ? sum / n : 0;
  // Round to 3 decimals to avoid float noise; integer results print without a dot.
  return { text: `${Math.round(val * 1000) / 1000}`, skipped };
}

/** Bucket rows by a group field. `"page"` groups by the source page; any other
 *  field groups by that property's value (absent → "(none)"). Insertion order is
 *  preserved (first-seen key first). */
export function groupRows(set: AggRow[], field: string): Map<string, AggRow[]> {
  const map = new Map<string, AggRow[]>();
  for (const r of set) {
    const key = field === "page" ? r.page : r.props[field] ?? "(none)";
    const bucket = map.get(key);
    if (bucket) bucket.push(r);
    else map.set(key, [r]);
  }
  return map;
}

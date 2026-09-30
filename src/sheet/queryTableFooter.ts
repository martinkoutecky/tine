import type { QueryFooterCell } from "../components/SheetAggregateFooter";
import { querySummary, type QueryAggFn } from "../editor/queryAggregate";
import type { QueryStatistics, ViewSettings } from "../editor/queryIr";
import type { FieldId } from "./fields";

/** The current authored view and its guarded Display writer, plus complete
 * statistics and the executed view they belong to. An absent statistics
 * answer is not zero. Edits keep the current view's other settings. */
export interface QueryDisplayControl {
  view: ViewSettings;
  statistics?: QueryStatistics;
  statisticsView?: ViewSettings;
  apply: (next: ViewSettings) => void;
}

/** One query property's footer; builtin/computed columns return undefined.
 * Formats supplied backend statistics through querySummary, never table rows.
 * Missing/unmatched statistics yield blank text; backend markers stay visible.
 * Edits go through Display and preserve ordered repeated entries. Work is
 * O(supplied aggregates), with no graph read, query run or persistence door.
 * Exemplar for I-12: docs/contracts/query-table-footer.md. */
export function queryTableFooter(control: QueryDisplayControl | undefined, field: FieldId): QueryFooterCell | undefined {
  if (!control || !field.startsWith("prop:")) return undefined;
  const key = field.slice(5);
  // The existing query aggregate property grammar cannot carry these keys.
  if (!key || key !== key.trim() || /[=;\0\r\n]/.test(key)) return undefined;
  const fn = (): QueryAggFn | null =>
    (control.statisticsView?.aggregates ?? control.view.aggregates ?? []).find(([k]) => k === key)?.[1] ?? null;
  return {
    get fn() { return fn(); },
    get text() {
      const statistics = control.statistics;
      const op = fn();
      const at = statistics?.aggregates.findIndex(([field, f]) => field === key && f === op) ?? -1;
      return querySummary({ statistics })?.overall[at]?.text ?? "";
    },
    set: (next) => {
      const entries = [...(control.view.aggregates ?? [])];
      const at = entries.findIndex(([k]) => k === key);
      if (next === null) {
        if (at < 0) return;
        entries.splice(at, 1);
      } else if (at >= 0) entries[at] = [key, next];
      else entries.push([key, next]);
      control.apply({ ...control.view, aggregates: entries });
    },
  };
}

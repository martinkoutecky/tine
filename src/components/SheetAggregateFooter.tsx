import { Show, type JSX } from "solid-js";
import { aggregate, AGGREGATE_FNS, AGGREGATE_LABELS, type AggregateFn } from "../sheet/aggregate";
import type { FieldValue } from "../sheet/fields";
import type { QueryAggFn } from "../editor/queryAggregate";
import { setColumnAggregate } from "../sheet/mutations";
import { openActionContextMenu, type ContextMenuAction } from "../ui";

export function SheetAggregateCornerToggle(props: {
  active: boolean;
  onClick: (e: MouseEvent) => void;
}): JSX.Element {
  return (
    <button
      type="button"
      class="sheet-aggregate-corner-toggle"
      classList={{ "sheet-aggregate-corner-toggle-active": props.active }}
      title={props.active ? "Hide aggregate footer" : "Show aggregate footer"}
      aria-pressed={props.active ? "true" : "false"}
      onPointerDown={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={props.onClick}
    >
      Σ
    </button>
  );
}

/** The QUERY face's footer, which is a different property vocabulary living in
 *  the same cell (contract §4).
 *
 *  `tine.col-aggregates` is shared ground: the query reader understands
 *  count/sum/avg and an ORDERED list, the sheet footer understands seventeen
 *  names keyed into a `Map`. A query column may therefore never be offered the
 *  sheet's list — `avg` has no sheet implementation, and the other fourteen
 *  names have no query reader — so a query cell states its own choice, its own
 *  already-computed value, and its own writer. */
export interface QueryFooterCell {
  fn: QueryAggFn | null;
  /** The value, computed by the caller through the ONE query summary. */
  text: string;
  set: (fn: QueryAggFn | null) => void;
}

const QUERY_AGGREGATE_LABELS: readonly (readonly [QueryAggFn, string])[] = [
  ["count", "Count"],
  ["sum", "Sum"],
  ["avg", "Average"],
];

export function SheetAggregateFooterCell(props: {
  ownerId: string;
  columnKey: string;
  fn: AggregateFn | null;
  values: readonly (FieldValue | string | null | undefined)[];
  showEmpty?: boolean;
  stickyLeft?: boolean;
  query?: QueryFooterCell;
}): JSX.Element {
  const stop = (e: Event) => e.stopPropagation();
  // The picker is an in-DOM portaled menu, NOT a native <select>: in
  // WebKitGTK a select's popup is a separate native window that steals
  // focus, so the select blurs (and any blur-teardown kills the popup)
  // the instant it opens.
  const openMenu = (e: MouseEvent) => {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const query = props.query;
    if (query) {
      const item = (label: string, value: QueryAggFn | null): ContextMenuAction => ({
        label: (query.fn ?? null) === value ? `✓ ${label}` : label,
        run: () => query.set(value),
      });
      openActionContextMenu(rect.left, rect.bottom + 4, [
        item("None", null),
        ...QUERY_AGGREGATE_LABELS.map(([fn, label]) => item(label, fn)),
      ]);
      return;
    }
    const item = (label: string, value: AggregateFn | null): ContextMenuAction => ({
      label: (props.fn ?? null) === value ? `✓ ${label}` : label,
      run: () => setColumnAggregate(props.ownerId, props.columnKey, value),
    });
    openActionContextMenu(rect.left, rect.bottom + 4, [
      item("None", null),
      ...AGGREGATE_FNS.map((fn) => item(AGGREGATE_LABELS[fn], fn)),
    ]);
  };
  const activeFn = () => (props.query ? props.query.fn : props.fn);
  const activeLabel = () => {
    const query = props.query;
    if (query) return QUERY_AGGREGATE_LABELS.find(([fn]) => fn === query.fn)?.[1] ?? "";
    return props.fn ? AGGREGATE_LABELS[props.fn] : "";
  };

  return (
    <div
      class="sheet-cell sheet-footer-cell"
      classList={{ "sheet-sticky-left": !!props.stickyLeft }}
      onPointerDown={stop}
      onMouseDown={stop}
      onClick={stop}
    >
      <Show
        when={activeFn()}
        fallback={
          <Show when={props.showEmpty}>
            <button class="sheet-aggregate-add" title="Add aggregate" onClick={openMenu}>
              Σ
            </button>
          </Show>
        }
      >
        {(fn) => (
          <button
            class="sheet-aggregate-value"
            title={`Change aggregate (${activeLabel()})`}
            onClick={openMenu}
          >
            {props.query ? props.query.text : aggregate(fn() as AggregateFn, props.values)}
          </button>
        )}
      </Show>
    </div>
  );
}

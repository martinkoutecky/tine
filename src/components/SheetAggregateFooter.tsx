import { For, Show, createSignal, type JSX } from "solid-js";
import { aggregate, AGGREGATE_FNS, AGGREGATE_LABELS, type AggregateFn } from "../sheet/aggregate";
import type { FieldValue } from "../sheet/fields";
import { setColumnAggregate } from "../sheet/mutations";

export function SheetAggregateFooterCell(props: {
  ownerId: string;
  columnKey: string;
  fn: AggregateFn | null;
  values: readonly (FieldValue | string | null | undefined)[];
  showEmpty?: boolean;
  stickyLeft?: boolean;
}): JSX.Element {
  const [editing, setEditing] = createSignal(false);
  const stop = (e: Event) => e.stopPropagation();
  const commit = (value: string) => {
    setColumnAggregate(props.ownerId, props.columnKey, value ? (value as AggregateFn) : null);
    setEditing(false);
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
        when={editing()}
        fallback={
          <Show
            when={props.fn}
            fallback={
              <Show when={props.showEmpty}>
                <button
                  class="sheet-aggregate-add"
                  title="Add aggregate"
                  onClick={() => setEditing(true)}
                >
                  Σ
                </button>
              </Show>
            }
          >
            {(fn) => (
              <button
                class="sheet-aggregate-value"
                title={`Change aggregate (${AGGREGATE_LABELS[fn()]})`}
                onClick={() => setEditing(true)}
              >
                {aggregate(fn(), props.values)}
              </button>
            )}
          </Show>
        }
      >
        <select
          class="sheet-aggregate-select"
          autofocus
          value={props.fn ?? ""}
          onChange={(e) => commit(e.currentTarget.value)}
          onBlur={() => setEditing(false)}
        >
          <option value="">None</option>
          <For each={AGGREGATE_FNS}>
            {(fn) => <option value={fn}>{AGGREGATE_LABELS[fn]}</option>}
          </For>
        </select>
      </Show>
    </div>
  );
}

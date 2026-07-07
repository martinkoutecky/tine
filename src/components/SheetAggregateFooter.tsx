import { For, Show, createSignal, onCleanup, type JSX } from "solid-js";
import { aggregate, AGGREGATE_FNS, AGGREGATE_LABELS, type AggregateFn } from "../sheet/aggregate";
import type { FieldValue } from "../sheet/fields";
import { setColumnAggregate } from "../sheet/mutations";

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

export function SheetAggregateFooterCell(props: {
  ownerId: string;
  columnKey: string;
  fn: AggregateFn | null;
  values: readonly (FieldValue | string | null | undefined)[];
  showEmpty?: boolean;
  stickyLeft?: boolean;
  onEditingChange?: (editing: boolean) => void;
}): JSX.Element {
  const [editing, setEditing] = createSignal(false);
  let blurTimer = 0;
  const stop = (e: Event) => e.stopPropagation();
  const setEditingState = (next: boolean) => {
    if (editing() === next) return;
    setEditing(next);
    props.onEditingChange?.(next);
  };
  const commit = (value: string) => {
    if (blurTimer) {
      window.clearTimeout(blurTimer);
      blurTimer = 0;
    }
    setColumnAggregate(props.ownerId, props.columnKey, value ? (value as AggregateFn) : null);
    setEditingState(false);
  };
  const closeAfterBlur = () => {
    if (blurTimer) window.clearTimeout(blurTimer);
    blurTimer = window.setTimeout(() => {
      blurTimer = 0;
      setEditingState(false);
    }, 0);
  };

  onCleanup(() => {
    if (blurTimer) window.clearTimeout(blurTimer);
    if (editing()) props.onEditingChange?.(false);
  });

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
                  onClick={() => setEditingState(true)}
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
                onClick={() => setEditingState(true)}
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
          onBlur={closeAfterBlur}
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

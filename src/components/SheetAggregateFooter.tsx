import { Show, type JSX } from "solid-js";
import { aggregate, AGGREGATE_FNS, AGGREGATE_LABELS, type AggregateFn } from "../sheet/aggregate";
import type { FieldValue } from "../sheet/fields";
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

export function SheetAggregateFooterCell(props: {
  ownerId: string;
  columnKey: string;
  fn: AggregateFn | null;
  values: readonly (FieldValue | string | null | undefined)[];
  showEmpty?: boolean;
  stickyLeft?: boolean;
}): JSX.Element {
  const stop = (e: Event) => e.stopPropagation();
  // The picker is an in-DOM portaled menu, NOT a native <select>: in
  // WebKitGTK a select's popup is a separate native window that steals
  // focus, so the select blurs (and any blur-teardown kills the popup)
  // the instant it opens.
  const openMenu = (e: MouseEvent) => {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const item = (label: string, value: AggregateFn | null): ContextMenuAction => ({
      label: (props.fn ?? null) === value ? `✓ ${label}` : label,
      run: () => setColumnAggregate(props.ownerId, props.columnKey, value),
    });
    openActionContextMenu(rect.left, rect.bottom + 4, [
      item("None", null),
      ...AGGREGATE_FNS.map((fn) => item(AGGREGATE_LABELS[fn], fn)),
    ]);
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
        when={props.fn}
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
            title={`Change aggregate (${AGGREGATE_LABELS[fn()]})`}
            onClick={openMenu}
          >
            {aggregate(fn(), props.values)}
          </button>
        )}
      </Show>
    </div>
  );
}

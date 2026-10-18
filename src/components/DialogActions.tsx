import { Show, type JSX } from "solid-js";

/**
 * The one button row of every confirm/cancel surface (D-20261018-UI1).
 *
 * Order is fixed here, not by the caller: auxiliary actions at the leading end,
 * then Cancel, then the primary action at the trailing end. A caller names which
 * slot a button belongs to; it cannot place it. DOM order equals visual order
 * (no `row-reverse` / `order`, which `dialogActions.guard.test.ts` enforces), so
 * a future per-platform flip is one rule in `.dialog-actions`. Cancel and the
 * primary action form one non-wrapping group (`.dialog-actions-main`): in a
 * narrow dialog the auxiliary actions wrap above it, Cancel never drops onto a
 * row of its own (`scripts/shot-action-placement-layout.mjs` asserts this).
 *
 * Unlike actions are never paired: a destructive or unrelated action does not
 * sit beside Cancel/primary; give it the `aux` slot (leading end) or a separate
 * row.
 */
export function DialogActions(props: {
  /** Leading-end auxiliary actions (Reload, Clear, Undo, ...). */
  aux?: JSX.Element;
  /** Cancel / Close / Go back: immediately left of the primary action. */
  cancel?: JSX.Element;
  /** The confirming action, at the trailing end. */
  primary: JSX.Element;
  /** Extra class for per-dialog spacing or sizing; never for ordering. */
  class?: string;
}) {
  return (
    <div class={`dialog-actions${props.class ? ` ${props.class}` : ""}`}>
      <Show when={props.aux}>
        <span class="dialog-actions-aux">{props.aux}</span>
      </Show>
      <span class="dialog-actions-main">
        {props.cancel}
        {props.primary}
      </span>
    </div>
  );
}

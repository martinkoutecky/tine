import { createSignal } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";
import { dismissToast } from "./toasts";
import { reportUiFailure } from "./uiFailure";
import type { ConfigProblem } from "./backendTypes";

/** Whether the current binding's config is unreadable, supplied by Rust.
 * O(1), no I/O; the cause is logged, never retained as graph-bearing UI state. */
const [unreadable, setUnreadable] = createSignal(false);
export const graphConfigUnreadable = unreadable;
let notice: number | null = null;

/** Adopt the store's read problem at graph open or live publication. A repeated
 * problem keeps one notice; recovery and binding retirement dismiss it. The
 * page's intrinsic Org restriction stays separate. O(visible toasts), no page
 * fetch, persistence or config inspection. Callers validate live bindings. */
export function setGraphConfigProblem(next: ConfigProblem | null): void {
  const previous = graphConfigUnreadable();
  setUnreadable(next !== null);
  if (next && !previous) notice = reportUiFailure("config-read", next);
  if (!next && notice !== null) { dismissToast(notice); notice = null; }
}

clearOnBindingInvalidated(() => setGraphConfigProblem(null));

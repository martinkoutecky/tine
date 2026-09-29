import { createSignal, type Accessor } from "solid-js";
import { backend } from "./backend";
import { graphEpoch } from "./graphSession";
import { pushToast } from "./toasts";

// A store reset also invalidates work before graphEpoch is published. The native
// generation pins calls which wait inside the backend before invoking Tauri.
let resetGeneration = 0;
const scopedClears = new Set<() => void>();

export interface Binding {
  readonly epoch: number;
  readonly resetGeneration: number;
  readonly backendGeneration: number;
}

export function captureBinding(): Binding {
  const generation = backend().graphBindingGeneration?.() ?? 0;
  return {
    epoch: graphEpoch(),
    resetGeneration,
    backendGeneration: generation,
  };
}

/** The identity of the current graph binding alone: the store-reset generation
 * and the backend binding generation, WITHOUT the display epoch. Display-only
 * repaints (page rename, typography, journal-title format) bump `graphEpoch`
 * but keep the same graph, so work that must survive them (a plugin command in
 * flight) compares this instead of `stillBound`. O(1); never throws. */
export function bindingIdentity(): string {
  return `${resetGeneration}:${backend().graphBindingGeneration?.() ?? 0}`;
}

export function stillBound(binding: Binding): boolean {
  return binding.epoch === graphEpoch()
    && binding.resetGeneration === resetGeneration
    && binding.backendGeneration === (backend().graphBindingGeneration?.() ?? 0);
}

/** Retire every binding (store reset: graph switch, restore): bumps a
 * module-private, non-reactive reset generation (not `graphEpoch`) and runs every
 * registered clear in registration order: each `graphScopedSignal`'s (closing
 * its popup) and each `clearOnBindingInvalidated` callback (e.g. the outline
 * selection). Each clear is isolated: one that throws does not stop the others
 * or the caller's remaining reset. Each failure is logged as fixed text (I-5)
 * and together they show one error toast with the fixed family `binding.clear`
 * (I-9). Never throws. Idempotent.
 * O(number of registered clears). */
export function invalidateBinding(): void {
  resetGeneration++;
  let failures = 0;
  for (const clear of scopedClears) {
    try {
      clear();
    } catch {
      failures++;
      console.error("binding.clear: a graph-scoped clear failed");
    }
  }
  if (failures > 0) {
    pushToast(`Some state of the previous graph could not be cleared (binding.clear, ${failures} failed).`, "error");
  }
}

/** I-20: module state that names graph content (a block id, page name or
 * selection) and outlives the component that set it, such as a popup, editor
 * or menu target mounted at the app root. Runtime block ids are derived from
 * (page path, sibling position), so the same id exists in every graph; a
 * target that survived a graph switch would write into the new graph.
 * The value carries the binding captured when it was set. It reads `null` once
 * that binding is stale and is cleared by `invalidateBinding`, so the popup
 * closes on a switch. A writer that holds the value it was opened with checks
 * `signal() === value` before writing and otherwise calls `refuseStaleWrite`.
 * Exemplar: `formulaEditor` (src/ui.ts) and FormulaEditor's `save`.
 * Reads are O(1) and track `graphEpoch`; a change of the backend binding
 * generation alone is seen on the next read but does not notify (a graph switch
 * also clears the value through `invalidateBinding`). */
export function graphScopedSignal<T>(): readonly [Accessor<T | null>, (value: T | null) => void] {
  const [held, setHeld] = createSignal<{ value: T; binding: Binding } | null>(null);
  scopedClears.add(() => setHeld(null));
  const read = () => {
    const current = held();
    return current && stillBound(current.binding) ? current.value : null;
  };
  const write = (value: T | null) => setHeld(value === null ? null : { value, binding: captureBinding() });
  return [read, write] as const;
}

/** Register a clear for existing module state of the graph-scoped class that
 * is not a `graphScopedSignal` (e.g. the outline selection). The registration
 * is permanent (call it at module level); the same function registers once.
 * A throwing clear is isolated and surfaced by `invalidateBinding`. */
export function clearOnBindingInvalidated(clear: () => void): void {
  scopedClears.add(clear);
}

/** The visible refusal for a write whose popup outlived its graph. */
export function refuseStaleWrite(what: string): void {
  pushToast(`${what} was not saved: it was opened in a graph that is no longer open.`, "error");
}

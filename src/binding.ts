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

export function stillBound(binding: Binding): boolean {
  return binding.epoch === graphEpoch()
    && binding.resetGeneration === resetGeneration
    && binding.backendGeneration === (backend().graphBindingGeneration?.() ?? 0);
}

/** Retire every binding (store reset: graph switch, restore) and close every
 * graph-scoped popup. O(number of graph-scoped signals). */
export function invalidateBinding(): void {
  resetGeneration++;
  for (const clear of scopedClears) clear();
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
 * Reads are O(1) and track `graphEpoch`. */
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
 * is not a `graphScopedSignal` (e.g. the outline selection). */
export function clearOnBindingInvalidated(clear: () => void): void {
  scopedClears.add(clear);
}

/** The visible refusal for a write whose popup outlived its graph. */
export function refuseStaleWrite(what: string): void {
  pushToast(`${what} was not saved: it was opened in a graph that is no longer open.`, "error");
}

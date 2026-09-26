import { backend } from "./backend";
import { graphEpoch } from "./ui";

// A store reset also invalidates work before graphEpoch is published. The native
// generation pins calls which wait inside the backend before invoking Tauri.
let resetGeneration = 0;

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

export function invalidateBinding(): void {
  resetGeneration++;
}

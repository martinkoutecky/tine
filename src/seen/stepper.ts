// The routed page whose seen notice is on screen offers Previous / Next change
// to the command palette too (D-20261029-SS). The notice registers a step
// function under its page while it is mounted; the commands look it up by the
// focused pane's page. Component lifetime, no graph content.
import { createSignal } from "solid-js";
import { clearOnBindingInvalidated } from "../binding";
import { keyOf } from "./displayed";

const steppers = new Map<string, (direction: 1 | -1) => void>();
const [revision, setRevision] = createSignal(0);
// The notices are rebuilt with the graph; a stale closure must not outlive its binding.
clearOnBindingInvalidated(() => { steppers.clear(); setRevision((n) => n + 1); });

/** Register the notice's stepper for `pageName`; returns its cleanup. */
export function registerSeenStepper(pageName: string, step: (direction: 1 | -1) => void): () => void {
  const key = keyOf(pageName);
  steppers.set(key, step);
  setRevision((n) => n + 1);
  return () => {
    if (steppers.get(key) === step) steppers.delete(key);
    setRevision((n) => n + 1);
  };
}

/** Does `pageName` show a seen notice with changes to step through? Reactive. */
export function seenStepAvailable(pageName: string | null): boolean {
  revision();
  return pageName !== null && steppers.has(keyOf(pageName));
}

export function stepSeenChange(pageName: string | null, direction: 1 | -1): void {
  if (pageName !== null) steppers.get(keyOf(pageName))?.(direction);
}

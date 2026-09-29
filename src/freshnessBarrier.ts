import { createSignal } from "solid-js";

// Family 10 (master d56219d73): a focus-driven disk rescan must finish before a
// new edit starts, so a keystroke never lands on a page that is about to be
// replaced by what changed on disk while the window was away. Dependency-free
// so the focus coordinator and the editor controller share it without a cycle.
const [freshnessPending, setFreshnessPending] = createSignal(false);
const [freshnessVisible, setFreshnessVisible] = createSignal(false);
let deferredEditorStart: (() => void) | null = null;
let visibilityTimer: ReturnType<typeof setTimeout> | null = null;

export { freshnessPending, freshnessVisible };

export function beginFreshnessBarrier(): void {
  setFreshnessPending(true);
  // A fast rescan never flashes a notice; a slow one says what the wait is.
  visibilityTimer ??= setTimeout(() => {
    visibilityTimer = null;
    if (freshnessPending()) setFreshnessVisible(true);
  }, 120);
}

export function endFreshnessBarrier(): void {
  setFreshnessPending(false);
  if (visibilityTimer !== null) clearTimeout(visibilityTimer);
  visibilityTimer = null;
  setFreshnessVisible(false);
  const deferred = deferredEditorStart;
  deferredEditorStart = null;
  deferred?.();
}

/** Defer the newest editor activation until the rescan's page state is
 *  installed. Returns true when the caller must stop now. */
export function deferEditorStartUntilFresh(start: () => void): boolean {
  if (!freshnessPending()) return false;
  deferredEditorStart = start;
  return true;
}

/** A textarea can keep focus across suspend, so block input at the window
 *  too; deferring `startEditing` alone covers only new activations. */
export function installFreshnessInputGate(): void {
  if (typeof window === "undefined") return;
  const block = (event: Event) => {
    if (!freshnessPending()) return;
    event.preventDefault();
    event.stopImmediatePropagation();
  };
  for (const type of ["beforeinput", "compositionstart", "keydown"]) window.addEventListener(type, block, true);
}

import { pageByName } from "./model";
import { createSignal, type Signal } from "solid-js";
import { invalidatePageRead } from "./readAdmission";

// A generation identifies one exact loaded page instance. It is deliberately
// frontend-only and monotonic across resets: a later page with the same name and
// path must never satisfy a cut payload, an undo entry or a Concord review
// captured from an evicted/deleted/rebound instance (GH #305; R7).
let pageInstanceClock = 0;
export const pageInstanceGenerations = new Map<string, number>();
// A retained outline no longer publishes its reload by replacing the page
// object. Readers of that exact loaded page still observe its generation.
// Weak ownership follows the page object, with no graph-wide invalidation.
const instanceSignals = new WeakMap<object, Signal<number>>();

export function activatePageInstance(name: string): number {
  const generation = ++pageInstanceClock;
  pageInstanceGenerations.set(name, generation);
  const page = pageByName(name);
  if (page) instanceSignals.get(page)?.[1](generation);
  return generation;
}

export function retirePageInstance(name: string): void {
  invalidatePageRead(name);
  ++pageInstanceClock;
  pageInstanceGenerations.delete(name);
}

/** Move an instance's generation to its new name (a title-identity rename keeps the instance). */
export function rekeyPageInstance(oldName: string, newName: string): void {
  const generation = pageInstanceGenerations.get(oldName);
  pageInstanceGenerations.delete(oldName);
  if (generation !== undefined) pageInstanceGenerations.set(newName, generation);
}

/** Current exact loaded-page generation, or null when that page is absent. */
export function pageInstanceGeneration(name: string): number | null {
  const page = pageByName(name);
  if (!page) return null;
  // Direct setDoc page seeding is supported by model tests and small embedded
  // surfaces; lazily bind it to the same invariant as loader-created pages.
  const generation = pageInstanceGenerations.get(name) ?? activatePageInstance(name);
  let signal = instanceSignals.get(page);
  if (!signal) { signal = createSignal(generation); instanceSignals.set(page, signal); }
  return signal[0]();
}

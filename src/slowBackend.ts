// Backend commands that have passed the slow threshold and not yet settled.
//
// Tine already records THAT a command was slow (the flight recorder, GH #343);
// this is the same fact made readable by the UI, so a region that fails while
// the backend is busy can say so ("waiting on 2 operations for 37s") instead of
// leaving the user to read a blank window as lost notes (GH #332). Module-level
// because the reader (FailureBoundary) is unrelated to the caller (the backend
// call funnel).

import { createSignal } from "solid-js";

const inFlight = new Map<number, number>();
let seq = 0;
const [revision, bump] = createSignal(0, { equals: false });

export interface SlowBackendState {
  /** Commands over the slow threshold that have not returned. */
  count: number;
  /** Milliseconds the longest-running of them has been waiting. */
  longestMs: number;
}

/** Register a command as slow, started at `startedAt` (performance.now()).
 * Returns the function that marks it settled. O(1). */
export function markCommandSlow(startedAt: number): () => void {
  const ticket = ++seq;
  inFlight.set(ticket, startedAt);
  bump(0);
  return () => {
    if (inFlight.delete(ticket)) bump(0);
  };
}

/** Reactive: re-reads whenever a command crosses or leaves the slow threshold.
 * O(slow commands in flight). */
export function slowBackendState(): SlowBackendState {
  revision();
  const now = performance.now();
  let longestMs = 0;
  for (const startedAt of inFlight.values()) longestMs = Math.max(longestMs, now - startedAt);
  return { count: inFlight.size, longestMs: Math.round(longestMs) };
}

export function resetSlowBackendStateForTests(): void {
  inFlight.clear();
  bump(0);
}

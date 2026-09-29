import type { Resource } from "solid-js";
import { dbg } from "./debug";

/**
 * Reading a Solid resource is this app's most common way to throw into render
 * (master c5279d186, GH #490/#332).
 *
 * `read()` (solid-js 1.9.13, dist/solid.js:318-322) does `if (err !== undefined
 * && !pr) throw err;`, and so does the `latest` getter (:390-397). So `data()`
 * on a rejected resource throws, and `runUpdates` discards the pending effect
 * queue before `handleError` runs — one failed fetch used to blank a region.
 * `src/components/FailureBoundary.tsx` stops the rethrow and offers a Retry; it
 * cannot stop the throw happening at whatever granularity the nearest seam has.
 *
 * Nearly every call site here had ALREADY written what to do without a value
 * (`icons() ?? {}`, `contentDays() ?? []`, `preview()?.blocks ?? []`), and none
 * of those branches could run on a rejection because the read threw first.
 * `readOr` consults `resource.error`, which never throws, and hands the site the
 * fallback it already had.
 *
 * `what` is a fixed label chosen in source, never a page name, path or query.
 * The record goes to the opt-in debug log only (`dbg`); og's always-on flight
 * recorder is a closed vocabulary (ADR 0058) and takes no new kind for this.
 *
 * This is NOT a way to swallow failures that matter. Where an empty value would
 * tell the user something untrue ("no references" when we could not load them),
 * the site must also SHOW the failure: `ResourceFailure` is the shared row.
 * Cost: O(1) per read; one debug line per distinct failure.
 */
export function readOr<T, F>(resource: Resource<T>, fallback: F, what: string): T | F {
  if (resource.error !== undefined) {
    report(what, resource.error);
    return fallback;
  }
  return resource() as T;
}

/** `readOr` over `resource.latest`: the value that survives a refetch, so a
 * reload does not flash empty. `latest` throws on a rejection exactly as
 * `read()` does (dist/solid.js:390-397). */
export function readLatestOr<T, F>(resource: Resource<T>, fallback: F, what: string): T | F {
  if (resource.error !== undefined) {
    report(what, resource.error);
    return fallback;
  }
  return resource.latest as T;
}

/** Failures already recorded, so re-rendering does not repeat one. */
const recorded = new Set<string>();

function report(what: string, error: unknown): void {
  let message: string;
  try {
    message = String(error);
  } catch {
    message = "";
  }
  const key = `${what} ${message}`;
  if (recorded.has(key)) return;
  recorded.add(key);
  dbg(`resource failed: ${what}: ${message}`);
}

/** Test seam: the dedupe set is module state and outlives a single test. */
export function resetResourceReportsForTests(): void {
  recorded.clear();
}

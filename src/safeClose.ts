export type SafeClosePrepareResult = "accepted" | "rejected" | "in_flight";

/** Why the close is asking the user to discard their work.
 *
 *  `failed` — the flush ran to completion and did not land: an unresolved
 *  conflict, or a save the backend refused. Waiting longer cannot help.
 *  `still-saving` — the flush is STILL RUNNING and may yet succeed. The two
 *  must not be worded alike: offering to discard work that is seconds from
 *  durable, in the same sentence used for work that can never be saved, invites
 *  the user to throw away a write in progress. (Direct Files data-safety audit,
 *  2026-08-09, finding 11.) */
export type DiscardReason = "failed" | "still-saving";

export interface SafeCloseDeps {
  blurActive(): void;
  endEdit(): void;
  flushPdfWork(): Promise<boolean>;
  flushAll(): Promise<boolean>;
  confirmDiscard(reason: DiscardReason): Promise<boolean>;
  flushSession(): Promise<void>;
  setTransition(active: boolean): void;
  notifyPdfFailure(): void;
  notifyStillSaving(): void;
  notifyConfirmationFailure(): void;
  runBounded?<T>(operation: Promise<T>, timeoutMs: number, fallback: T): Promise<T>;
}

/** How long the close waits in silence before telling the user anything. */
const FLUSH_SOFT_TIMEOUT_MS = 4_000;
/** …and how much longer it keeps waiting for a save that is still running.
 *
 *  A slow or network filesystem, a large batch of dirty pages, or a fsync behind
 *  a busy disk can all exceed the soft bound with nothing wrong. On the 1,045-file
 *  anonymized corpus a save averages ~7 ms, so the soft bound is already several
 *  hundred pages' worth: exceeding it means slow, not broken. */
const FLUSH_GRACE_TIMEOUT_MS = 26_000;

/** Distinct from `false`, which means the flush finished and did not land. */
const STILL_RUNNING = Symbol("flush-still-running");
type FlushOutcome = boolean | typeof STILL_RUNNING;

export interface SafeCloseCoordinator {
  prepare(): Promise<SafeClosePrepareResult>;
  reset(): void;
  inFlight(): boolean;
}

function runBounded<T>(operation: Promise<T>, timeoutMs: number, fallback: T): Promise<T> {
  return Promise.race([
    operation,
    new Promise<T>((resolve) => setTimeout(() => resolve(fallback), timeoutMs)),
  ]);
}

/** One persistence transaction shared by desktop window-close and Android root
 * Back. A pre-safe native preparation refusal resets this transition so a
 * later Back repeats the full close; after Android reaches the native-safe
 * state, an activity-exit failure deliberately remains shielded and retries
 * only that final handoff. */
export function createSafeCloseCoordinator(deps: SafeCloseDeps): SafeCloseCoordinator {
  let closing = false;
  const bounded = deps.runBounded ?? runBounded;

  const reset = () => {
    closing = false;
    deps.setTransition(false);
  };

  const prepare = async (): Promise<SafeClosePrepareResult> => {
    if (closing) return "in_flight";
    closing = true;
    deps.setTransition(true);
    let accepted = false;
    try {
      deps.blurActive();
      deps.endEdit();
      await Promise.resolve();

      let pdfSaved = false;
      try {
        // A pending PDF view-state timer is not visible to the page persistence
        // engine until it fires. Enroll and drain it first while this window's
        // current graph binding still owns every PDF mutation.
        pdfSaved = await bounded(deps.flushPdfWork(), FLUSH_SOFT_TIMEOUT_MS, false);
      } catch {
        pdfSaved = false;
      }
      if (!pdfSaved) {
        deps.notifyPdfFailure();
        return "rejected";
      }

      // Race the SAME promise twice rather than giving up at the soft bound: a
      // flush that is merely slow gets a grace period and usually lands, and the
      // user only ever sees a discard prompt that tells the truth about which
      // situation they are in.
      let outcome: FlushOutcome;
      const flushing = (() => {
        try {
          return deps.flushAll();
        } catch (error) {
          return Promise.reject(error);
        }
      })();
      try {
        outcome = await bounded<FlushOutcome>(flushing, FLUSH_SOFT_TIMEOUT_MS, STILL_RUNNING);
        if (outcome === STILL_RUNNING) {
          deps.notifyStillSaving();
          outcome = await bounded<FlushOutcome>(flushing, FLUSH_GRACE_TIMEOUT_MS, STILL_RUNNING);
        }
      } catch {
        outcome = false;
      }

      if (outcome !== true) {
        let discard = false;
        try {
          discard = await deps.confirmDiscard(outcome === STILL_RUNNING ? "still-saving" : "failed");
        } catch {
          deps.notifyConfirmationFailure();
          return "rejected";
        }
        if (!discard) return "rejected";
      }

      try {
        await bounded(deps.flushSession(), 1000, undefined);
      } catch {
        // Session state is best effort after graph content was saved or the user
        // explicitly accepted discarding it; preserve the established policy.
      }
      accepted = true;
      return "accepted";
    } finally {
      if (!accepted) reset();
    }
  };

  return { prepare, reset, inFlight: () => closing };
}

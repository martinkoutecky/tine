export type SafeClosePrepareResult = "accepted" | "rejected" | "in_flight";

export interface SafeCloseDeps {
  blurActive(): void;
  endEdit(): void;
  flushPdfWork(): Promise<boolean>;
  flushAll(): Promise<boolean>;
  confirmDiscard(reason: DiscardReason): Promise<boolean>;
  /** The user accepted losing work. Recorded (fixed reason, page count) so a
   *  run that discarded drafts is distinguishable in the diagnostic report
   *  (GH #540). Bounded to one second; its failure never blocks the close. */
  recordDiscard?(reason: DiscardReason): Promise<void>;
  /** The user chose to keep unsaved work: show where it is (GH #540). */
  onDiscardDeclined?(): void;
  flushSession(): Promise<void>;
  setTransition(active: boolean): void;
  notifyPdfFailure(): void;
  /** The page flush outlasted the soft bound and the close is waiting out the grace period. */
  notifyStillSaving(): void;
  notifyConfirmationFailure(): void;
  runBounded?<T>(operation: Promise<T>, timeoutMs: number, fallback: T): Promise<T>;
}

export interface SafeCloseCoordinator {
  prepare(): Promise<SafeClosePrepareResult>;
  reset(): void;
  inFlight(): boolean;
}

/** The flush fallback when saves were still running at a bound. Distinct from
 *  `false`, which means the flush finished and did not land. */
const STILL_RUNNING = Symbol("still-running");
/** How long the close waits in silence before saying it is still saving. */
const FLUSH_SOFT_TIMEOUT_MS = 4_000;
/** How much longer it keeps waiting on the SAME flush before offering discard:
 *  a slow or network drive, a busy fsync or many dirty pages exceed the soft
 *  bound with nothing wrong (master fea3c314b, Direct Files audit finding 11). */
const FLUSH_GRACE_TIMEOUT_MS = 26_000;

function runBounded<T>(operation: Promise<T>, timeoutMs: number, fallback: T): Promise<T> {
  return Promise.race([
    operation,
    new Promise<T>((resolve) => setTimeout(() => resolve(fallback), timeoutMs)),
  ]);
}

/** Create a close coordinator. prepare blurs/ends editing, waits up to four
 * seconds for the PDF flush and for the page flush; a page flush still running
 * then gets notifyStillSaving and a further 26-second grace on the same promise.
 * Only a flush that finished false, threw or outlasted the grace reaches
 * confirmDiscard ("failed" / "still-saving"), then a best-effort one-second
 * session flush. It returns
 * rejected, accepted or in_flight; accepted stays in flight until native close
 * succeeds. After native close failure the caller must reset before retrying.
 * Work follows pending PDF, page and session bytes within those waits. */
export function createSafeCloseCoordinator(deps: SafeCloseDeps): SafeCloseCoordinator {
  let closing = false;
  const transaction = {};
  const bounded = deps.runBounded ?? runBounded;

  const reset = () => {
    advanceRevision(transaction);
    closing = false;
    deps.setTransition(false);
  };

  const prepare = async (): Promise<SafeClosePrepareResult> => {
    if (closing) return "in_flight";
    closing = true;
    const owner = revisionOwner(transaction, advanceRevision(transaction));
    deps.setTransition(true);
    let accepted = false;
    try {
      deps.blurActive();
      deps.endEdit();
      await Promise.resolve();
      if (!owner()) return "rejected";

      let pdfSaved = false;
      try {
        // A pending PDF view-state timer is not visible to the page persistence
        // engine until it fires. Enroll and drain it first while this window's
        // current graph binding still owns every PDF mutation.
        const result = await writeOwned(owner, bounded(deps.flushPdfWork(), FLUSH_SOFT_TIMEOUT_MS, false));
        if (result.kind === "stale") return "rejected";
        pdfSaved = result.value;
      } catch {
        pdfSaved = false;
      }
      if (!pdfSaved) {
        deps.notifyPdfFailure();
        return "rejected";
      }

      let saved: boolean | typeof STILL_RUNNING = false;
      try {
        // Race the SAME flush twice: a merely slow save gets a grace period
        // instead of a discard prompt worded as if it could never land.
        const flushing = deps.flushAll();
        let result = await writeOwned(owner, bounded<boolean | typeof STILL_RUNNING>(flushing, FLUSH_SOFT_TIMEOUT_MS, STILL_RUNNING));
        if (result.kind === "stale") return "rejected";
        if (result.value === STILL_RUNNING) {
          deps.notifyStillSaving();
          result = await writeOwned(owner, bounded<boolean | typeof STILL_RUNNING>(flushing, FLUSH_GRACE_TIMEOUT_MS, STILL_RUNNING));
          if (result.kind === "stale") return "rejected";
        }
        saved = result.value;
      } catch {
        saved = false;
      }

      if (saved !== true) {
        const reason: DiscardReason = saved === STILL_RUNNING ? "still-saving" : "failed";
        let discard = false;
        try {
          const result = await readOwned(owner, deps.confirmDiscard(reason));
          if (result.kind === "stale") return "rejected";
          discard = result.value;
        } catch {
          deps.notifyConfirmationFailure();
          return "rejected";
        }
        if (!discard) {
          deps.onDiscardDeclined?.();
          return "rejected";
        }
        try {
          await writeOwned(owner, bounded(deps.recordDiscard?.(reason) ?? Promise.resolve(), 1000, undefined));
        } catch {
          dbg("close discard not recorded"); // diagnostics never block a confirmed close
        }
      }

      try {
        const result = await writeOwned(owner, bounded(deps.flushSession(), 1000, undefined));
        if (result.kind === "stale") return "rejected";
      } catch {
        // Session state is best effort after graph content was saved or the user
        // explicitly accepted discarding it; preserve the established policy.
      }
      accepted = true;
      return "accepted";
    } finally {
      if (!accepted && owner()) reset();
    }
  };

  return { prepare, reset, inFlight: () => closing };
}
import { advanceRevision, readOwned, revisionOwner, writeOwned } from "./owned";
import { dbg } from "./debug";
import type { DiscardReason } from "./backend";

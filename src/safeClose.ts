export type SafeClosePrepareResult = "accepted" | "rejected" | "in_flight";

export interface SafeCloseDeps {
  blurActive(): void;
  endEdit(): void;
  flushPdfWork(): Promise<boolean>;
  flushAll(): Promise<boolean>;
  confirmDiscard(): Promise<boolean>;
  /** The user accepted losing work. Recorded (fixed reason, page count) so a
   *  run that discarded drafts is distinguishable in the diagnostic report
   *  (GH #540). Bounded to one second; its failure never blocks the close. */
  recordDiscard?(reason: DiscardReason): Promise<void>;
  flushSession(): Promise<void>;
  setTransition(active: boolean): void;
  notifyPdfFailure(): void;
  notifyConfirmationFailure(): void;
  runBounded?<T>(operation: Promise<T>, timeoutMs: number, fallback: T): Promise<T>;
}

export interface SafeCloseCoordinator {
  prepare(): Promise<SafeClosePrepareResult>;
  reset(): void;
  inFlight(): boolean;
}

/** The flush fallback when saves were still running at the four-second bound. */
const STILL_RUNNING = Symbol("still-running");

function runBounded<T>(operation: Promise<T>, timeoutMs: number, fallback: T): Promise<T> {
  return Promise.race([
    operation,
    new Promise<T>((resolve) => setTimeout(() => resolve(fallback), timeoutMs)),
  ]);
}

/** Create a close coordinator. prepare blurs/ends editing, waits up to four
 * seconds each for PDF and page flushes, asks before discarding failed page
 * saves, then attempts a best-effort one-second session flush. It returns
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
        const result = await writeOwned(owner, bounded(deps.flushPdfWork(), 4000, false));
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
        const result = await writeOwned(owner, bounded<boolean | typeof STILL_RUNNING>(deps.flushAll(), 4000, STILL_RUNNING));
        if (result.kind === "stale") return "rejected";
        saved = result.value;
      } catch {
        saved = false;
      }

      if (saved !== true) {
        const reason: DiscardReason = saved === STILL_RUNNING ? "still-saving" : "failed";
        let discard = false;
        try {
          const result = await readOwned(owner, deps.confirmDiscard());
          if (result.kind === "stale") return "rejected";
          discard = result.value;
        } catch {
          deps.notifyConfirmationFailure();
          return "rejected";
        }
        if (!discard) return "rejected";
        try {
          await bounded(deps.recordDiscard?.(reason) ?? Promise.resolve(), 1000, undefined);
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

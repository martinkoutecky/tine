export type SafeClosePrepareResult = "accepted" | "rejected" | "in_flight";

export interface SafeCloseDeps {
  blurActive(): void;
  endEdit(): void;
  flushPdfWork(): Promise<boolean>;
  flushAll(): Promise<boolean>;
  confirmDiscard(): Promise<boolean>;
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

function runBounded<T>(operation: Promise<T>, timeoutMs: number, fallback: T): Promise<T> {
  return Promise.race([
    operation,
    new Promise<T>((resolve) => setTimeout(() => resolve(fallback), timeoutMs)),
  ]);
}

/** One persistence transaction shared by desktop window-close and Android root
 * Back.  Accepted transactions deliberately stay in-flight until the native
 * close succeeds; a failed native close must call reset() before retrying. */
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
        const result = await readOwned(owner, bounded(deps.flushPdfWork(), 4000, false));
        if (result.kind === "stale") return "rejected";
        pdfSaved = result.value;
      } catch {
        pdfSaved = false;
      }
      if (!pdfSaved) {
        deps.notifyPdfFailure();
        return "rejected";
      }

      let saved = false;
      try {
        const result = await readOwned(owner, bounded(deps.flushAll(), 4000, false));
        if (result.kind === "stale") return "rejected";
        saved = result.value;
      } catch {
        saved = false;
      }

      if (!saved) {
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
      }

      try {
        const result = await readOwned(owner, bounded(deps.flushSession(), 1000, undefined));
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
import { advanceRevision, readOwned, revisionOwner } from "./owned";

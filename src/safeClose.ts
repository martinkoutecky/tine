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
        pdfSaved = await bounded(deps.flushPdfWork(), 4000, false);
      } catch {
        pdfSaved = false;
      }
      if (!pdfSaved) {
        deps.notifyPdfFailure();
        return "rejected";
      }

      let saved = false;
      try {
        saved = await bounded(deps.flushAll(), 4000, false);
      } catch {
        saved = false;
      }

      if (!saved) {
        let discard = false;
        try {
          discard = await deps.confirmDiscard();
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

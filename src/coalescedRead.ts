import type { Owner } from "./owned";

/** Start a read immediately when idle. While it runs, retain only the newest
 * queued read; superseded callers settle with undefined. This bounds work to
 * one active read and one pending intent without adding a typing-pause timer.
 * Owners are checked again before queued work starts. Callers still guard the
 * result with readOwned (or their resource's request ownership).
 * O(1) scheduling/state per input; no result or graph cache. */
export function coalescedRead<T>() {
  type Job = { current: Owner; read(): Promise<T>; resolve(value: T | undefined): void; reject(error: unknown): void };
  let running = false;
  let pending: Job | undefined;
  const perform = async (job: Job) => {
    try { job.resolve(job.current() ? await job.read() : undefined); }
    catch (error) { job.reject(error); }
    finally {
      const next = pending; pending = undefined;
      if (next) void perform(next);
      else running = false;
    }
  };
  return {
    run(current: Owner, read: () => Promise<T>): Promise<T | undefined> {
      return new Promise((resolve, reject) => {
        const job = { current, read, resolve, reject };
        if (running) {
          pending?.resolve(undefined);
          pending = job;
        } else {
          running = true;
          void perform(job);
        }
      });
    },
    cancel() { pending?.resolve(undefined); pending = undefined; },
  };
}

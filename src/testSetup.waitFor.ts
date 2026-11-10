import { vi } from "vitest";

// vi.waitFor samples its condition every `interval` and, at `timeout`, rejects
// with the last sample's error without looking again. A worker that is not
// scheduled across the deadline (a gate at nice 19 beside busy lanes) wakes to
// every expired timer at once, in expiry order: one sample, then the code's own
// timer that makes the condition true, then the deadline, which reports the
// stale sample (card HBaE8: `expected [] to include '引用文本'` with that label
// rendered). This is vitest 3's waitFor with one more sample at the deadline:
// the deadline is unchanged, and a condition still false at it still fails.
// It resolves in the same tick as vitest's: callers that dispatch events right
// after a wait see the microtask order they were written against.

// The real timers, taken before any test can install fake ones (as vitest's own).
const { setTimeout, setInterval, clearTimeout, clearInterval } = globalThis;

vi.waitFor = ((callback, options = {}) => {
  const { interval = 50, timeout = 1000 } = typeof options === "number" ? { timeout: options } : options;
  return new Promise((resolve, reject) => {
    let lastError: unknown;
    let pending = false;
    let timeoutId: ReturnType<typeof setTimeout> | undefined;
    let intervalId: ReturnType<typeof setInterval> | undefined;
    const settle = (result: unknown) => {
      clearTimeout(timeoutId);
      clearInterval(intervalId);
      resolve(result as never);
    };
    const sample = () => {
      try {
        const result = callback();
        if (result !== null && typeof result === "object" && typeof (result as PromiseLike<unknown>).then === "function") {
          pending = true;
          (result as PromiseLike<unknown>).then(
            value => { pending = false; settle(value); },
            error => { pending = false; lastError = error; });
          return false;
        }
        settle(result);
        return true;
      } catch (error) {
        lastError = error;
        return false;
      }
    };
    const check = () => {
      if (vi.isFakeTimers()) vi.advanceTimersByTime(interval);
      return !pending && sample();
    };
    if (check()) return;
    timeoutId = setTimeout(() => {
      clearInterval(intervalId);
      if (!pending && sample()) return;
      reject(lastError ?? new Error("Timed out in waitFor!"));
    }, timeout);
    intervalId = setInterval(check, interval);
  });
}) as typeof vi.waitFor;

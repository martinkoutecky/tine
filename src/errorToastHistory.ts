// The last error messages Tine showed in this session, for Settings → Help &
// diagnostics (Martin, 2026-09-29). An error toast is sticky and copyable, but
// once closed its text is gone; this keeps the newest few so a report can still
// be written after the toast was dismissed.
//
// Question answered: "which error messages did this session show, and when?"
// Fed by the one error-toast recorder hook (`recordErrorToastsWith`, wired in
// debug.ts). In memory only, by design: the messages may name pages or paths, so
// they never reach the flight recorder, the debug log store, storage or the
// backend. A guard test pins that this module imports nothing that could write.
//
// Cost: O(limit) per error; bounded at 20 short strings. No persisted record.
import { createSignal } from "solid-js";

export const ERROR_TOAST_HISTORY_LIMIT = 20;

export interface ErrorToastEntry {
  message: string;
  /** Occurrences of this exact text this session (shown "×N" above 1). */
  count: number;
  /** Epoch ms of the most recent occurrence. */
  at: number;
}

const [entries, setEntries] = createSignal<ErrorToastEntry[]>([]);

/** Newest first. Reactive. */
export const errorToastHistory = entries;

/** Note one error toast. An identical text already listed is moved to the top
 *  with its count raised (as the toast itself does); otherwise a new entry is
 *  added and the oldest beyond the limit is dropped. */
export function recordErrorToastText(message: string, at: number = Date.now()): void {
  const current = entries();
  const same = current.find((entry) => entry.message === message);
  const rest = current.filter((entry) => entry !== same);
  const next: ErrorToastEntry = { message, count: (same?.count ?? 0) + 1, at };
  setEntries([next, ...rest].slice(0, ERROR_TOAST_HISTORY_LIMIT));
}

/** Test-only. */
export function resetErrorToastHistoryForTests(): void {
  setEntries([]);
}

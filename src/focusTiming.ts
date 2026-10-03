// Focus-return timing (GH #623). Ellis's diagnostics showed no stall because
// the recorder kept only commands slower than 500 ms and his session was
// 70 s. This module names the numbers that DO show it: how long each page
// load command took (always, not only when slow), how long it took within ten
// seconds after a window focus return, and the phase split of one focus
// rescan. Every name is a source literal mirrored by `TIMING_NAMES` in
// src-tauri/src/flight.rs (a Rust test keeps the lists equal); the backend
// keeps one fixed-size histogram per name, so nothing here is unbounded and
// nothing carries a path, page name or query (I-5).
// Unit cost: one fire-and-forget IPC of two numbers per timed command call
// and five per focus rescan; no bytes are written to the graph.

/** Commands timed on every call (and again, apart, right after a focus return). */
export const TIMED_COMMANDS: ReadonlySet<string> = new Set([
  "get_page", "get_page_by_path", "get_backlinks", "journal_feed_page", "page_inventory", "search",
]);

/** The phases of one focus rescan, in the order they happen. */
export const FOCUS_PHASES = ["focus.ipc", "focus.wait", "focus.apply", "focus.total", "focus.banner"] as const;
export type FocusPhase = (typeof FOCUS_PHASES)[number];

/** A call this soon after a focus return is also kept apart as `.afterFocus`. */
export const AFTER_FOCUS_WINDOW_MS = 10_000;

let lastFocusReturnAt: number | null = null;

/** The window was returned to (focus or visibility): later page loads are
 *  "after focus" for the next [`AFTER_FOCUS_WINDOW_MS`]. */
export function noteFocusReturn(now = performance.now()): void {
  lastFocusReturnAt = now;
}

/** Reset the clock (tests). */
export function resetFocusClock(): void { lastFocusReturnAt = null; }

/** The timing names one finished command call is counted under. */
export function timingNamesForCommand(command: string, now = performance.now()): string[] {
  if (!TIMED_COMMANDS.has(command)) return [];
  const afterFocus = lastFocusReturnAt !== null && now - lastFocusReturnAt <= AFTER_FOCUS_WINDOW_MS;
  return afterFocus ? [command, `${command}.afterFocus`] : [command];
}

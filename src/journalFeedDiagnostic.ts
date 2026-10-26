import { recordDiagnostic } from "./debug";
import type { JournalFeedCause, JournalFeedReason } from "./backendTypes";

/** Only fixed categories leave this function. Never stringify arbitrary graph
 * objects or send the native rejection text to the always-on recorder (I-5).
 * Native I/O categories are recorded before the bridge stringifies them. */
export function journalFeedCause(error: unknown): JournalFeedCause {
  if (error == null) return "empty-rejection";
  return "other-rejection";
}

/** Best effort: recorder failure never affects the feed or its retry policy. */
export function recordFeedReason(journalFeedReason: JournalFeedReason, error?: unknown): void {
  const fields = journalFeedReason === "feed-error" || journalFeedReason === "template-error"
    ? { journalFeedReason, journalFeedCause: journalFeedCause(error) } : { journalFeedReason };
  void recordDiagnostic("journal_feed", fields);
}

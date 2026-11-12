import { t } from "./i18n";

export type ErrorFamily =
  | "conflict" | "deleted" | "twin" | "repeated" | "read-only" | "invalid-target"
  | "closed" | "not-found" | "asset-too-large" | "result-too-large" | "stale-graph-binding" | "io" | "rollback-incomplete" | "publication-incomplete"
  | "config-unreadable"
  | "title-identity"
  | "missing-native-name-range"
  | "alias-owner"
  | "unreadable-owner" | "graph-app-owner" | "conflict-copy-gone" | "unknown";

/** Classify exact Tauri wire tokens for control flow. Cost O(message length);
 * unknown strings are `unknown`, and human prose is never interpreted. */
export function errorFamily(error: unknown): ErrorFamily {
  const message = error instanceof Error ? error.message : String(error);
  if (/^io:[A-Za-z]+$/.test(message)) return "io";
  if (/^config-unreadable-after: [^\n]*\n/.test(message)) return "config-unreadable";
  const withDetail = /^(rollback-incomplete|publication-incomplete|config-unreadable):[\s\S]*$/.exec(message);
  if (withDetail) return withDetail[1] as "rollback-incomplete" | "publication-incomplete" | "config-unreadable";
  switch (message) {
    case "Q-TITLE": return "title-identity";
    case "F-LINK": return "missing-native-name-range";
    case "Q-OWNER": return "alias-owner";
    case "conflict":
    case "deleted":
    case "twin":
    case "repeated":
    case "read-only":
    case "invalid-target":
    case "closed":
    case "not-found":
    case "asset-too-large":
    case "result-too-large":
    case "stale-graph-binding":
    case "unreadable-owner":
    case "graph-app-owner":
    case "conflict-copy-gone":
      return message;
    default:
      return "unknown";
  }
}

/** The window's words for a `config-unreadable` refusal: the catalog sentence
 *  (never the read error), after what a partial operation already saved when
 *  the refusal came after part of it (Rust `after_partial`). O(message length). */
export function configUnreadableText(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  const after = /^config-unreadable-after: ([^\n]*)\n/.exec(message);
  return after ? t("ui-failure-config-read-after", { done: after[1] }) : t("ui-failure-config-read");
}

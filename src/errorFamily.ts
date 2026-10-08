export type ErrorFamily =
  | "conflict" | "deleted" | "twin" | "repeated" | "read-only" | "invalid-target"
  | "closed" | "not-found" | "asset-too-large" | "result-too-large" | "stale-graph-binding" | "io" | "rollback-incomplete" | "publication-incomplete"
  | "unreadable-owner" | "unknown";

/** Classify exact Tauri wire tokens for control flow. Cost O(message length);
 * unknown strings are `unknown`, and human prose is never interpreted. */
export function errorFamily(error: unknown): ErrorFamily {
  const message = error instanceof Error ? error.message : String(error);
  if (/^io:[A-Za-z]+$/.test(message)) return "io";
  const incomplete = /^(rollback-incomplete|publication-incomplete):[\s\S]*$/.exec(message);
  if (incomplete) return incomplete[1] as "rollback-incomplete" | "publication-incomplete";
  switch (message) {
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
      return message;
    default:
      return "unknown";
  }
}

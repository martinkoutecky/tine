export type ErrorFamily =
  | "conflict" | "deleted" | "twin" | "read-only" | "invalid-target"
  | "closed" | "asset-too-large" | "io" | "unknown";

/** Only fixed Tauri wire tokens carry control flow. Human prose is display only. */
export function errorFamily(error: unknown): ErrorFamily {
  const message = error instanceof Error ? error.message : String(error);
  if (/^io:[A-Za-z]+$/.test(message)) return "io";
  switch (message) {
    case "conflict":
    case "deleted":
    case "twin":
    case "read-only":
    case "invalid-target":
    case "closed":
    case "asset-too-large":
      return message;
    default:
      return "unknown";
  }
}

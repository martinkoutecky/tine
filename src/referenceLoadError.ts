import { errorFamily } from "./errorFamily";

/**
 * Shared classification for a failed Linked/Unlinked References fetch (port of
 * master 25f36f16dabd). The two panels used to map every failure except the
 * bounded-result token to "the backend request failed", which reads as
 * transient and throws away the backend's own explanation. Keep that text and
 * show it.
 */
export type ReferenceLoadError = {
  kind: "bounded" | "backend";
  /** The backend's own message, never discarded. */
  detail: string;
};

export function classifyReferenceLoadError(error: unknown): ReferenceLoadError {
  const detail = error instanceof Error ? error.message : String(error);
  return { kind: errorFamily(error) === "result-too-large" ? "bounded" : "backend", detail };
}

/** The sentence shown in the references banner. */
export function referenceLoadErrorMessage(error: ReferenceLoadError): string {
  if (error.kind === "bounded") return "Couldn’t load references: the bounded result limit was exceeded.";
  const detail = error.detail.trim();
  return detail.length > 0
    ? `Couldn’t load references: ${detail}`
    : "Couldn’t load references because the backend request failed.";
}

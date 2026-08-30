export const UI_STATE_LIFETIMES = [
  "device-preference",
  "graph-configuration",
  "graph-session",
  "transient-runtime",
] as const;

export type UiStateLifetime = (typeof UI_STATE_LIFETIMES)[number];

export interface PersistedPdfTarget {
  filename: string;
  label: string;
}

interface UiStateDecision {
  owner: string;
  lifetime: UiStateLifetime;
  resetTrigger: string;
  persistence: string;
}

/**
 * Route-owned graph-session state is serialized by session.ts's explicit route
 * serializer. This registry is intentionally empty now that the retired
 * top-level PDF pane has migrated into ordinary pane routes.
 */
export const graphSessionUiStateRegistry: Record<string, UiStateDecision> = {};

export function parsePersistedPdfTarget(value: unknown): PersistedPdfTarget | null {
  if (!value || typeof value !== "object") return null;
  const input = value as Record<string, unknown>;
  if (typeof input.filename !== "string" || !input.filename || input.filename.length > 4096) return null;
  if (typeof input.label !== "string" || input.label.length > 4096) return null;
  return { filename: input.filename, label: input.label };
}

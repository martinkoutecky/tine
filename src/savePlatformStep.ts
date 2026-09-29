/** The failed platform call behind a save failure (master 678830a086af;
 *  GH #538, #590): `io:InvalidInput` alone could not tell an Android
 *  no-replace-rename refusal from a failed temporary-file write. */

export interface SavePlatformStep {
  operation: string | null;
  osError: number | null;
}

/** `; <operation>, os error <n>` — both closed backend vocabulary. */
export function describeSavePlatformStep(step: SavePlatformStep | null): string {
  if (!step) return "";
  const parts = [step.operation, step.osError === null ? null : `os error ${step.osError}`]
    .filter((part): part is string => part !== null);
  return parts.length ? `; ${parts.join(", ")}` : "";
}

/** The backend's operation names are fixed strings such as
 *  `renameat2(RENAME_NOREPLACE)`; anything else is dropped rather than shown,
 *  so no path or page text can reach a toast (I-5). */
const SAVE_OPERATION = /^[A-Za-z0-9 ()|_.,-]{1,120}$/u;

export function readSavePlatformStep(failure: unknown): SavePlatformStep | null {
  if (!failure || typeof failure !== "object") return null;
  const record = failure as Record<string, unknown>;
  const operation = typeof record.operation === "string" && SAVE_OPERATION.test(record.operation)
    ? record.operation
    : null;
  const osError = typeof record.osError === "number" && Number.isSafeInteger(record.osError)
    ? record.osError
    : null;
  return operation === null && osError === null ? null : { operation, osError };
}

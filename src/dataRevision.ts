import { createSignal } from "solid-js";

// Kept below graphSession's binding dependencies so registration introduces no
// graphSession → binding → refresh-key initialization cycle.
let refreshKeys: ((hasSummary: boolean) => boolean) | undefined;
export function installDataRevisionRefresh(refresh: (hasSummary: boolean) => boolean): void { refreshKeys = refresh; }
export const [dataRev, setDataRev] = createSignal(0);
export function bumpDataRev(hasSummary = false): void {
  // Compatibility counter only: precise publications release their keys.
  if (refreshKeys?.(hasSummary) !== false) setDataRev(n => n + 1);
}

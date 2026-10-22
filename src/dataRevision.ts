import { createSignal } from "solid-js";

// Kept below graphSession's binding dependencies so registration introduces no
// graphSession → binding → refresh-key initialization cycle.
let refreshKeys: ((hasSummary: boolean) => void) | undefined;
export function installDataRevisionRefresh(refresh: (hasSummary: boolean) => void): void { refreshKeys = refresh; }
export const [dataRev, setDataRev] = createSignal(0);
export function bumpDataRev(hasSummary = false): void {
  refreshKeys?.(hasSummary);
  setDataRev(n => n + 1);
}

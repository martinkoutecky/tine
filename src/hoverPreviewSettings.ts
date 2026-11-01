// Device-local mouse-hover delay, in seconds (GH #824). Touch hold has its own
// gesture delay. Reads and writes use the shared preference ownership/rollback.
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { loadPreference, writePreference } from "./preferenceWrites";

const KEY = "hover_preview_delay_seconds";
const [delay, setDelay] = createSignal(1);
export const hoverPreviewDelaySeconds = delay;

const valid = (value: number) => Number.isFinite(value) && value >= 0;

/** Apply valid seconds and queue persistence. Failed writes roll back and toast. */
export function setHoverPreviewDelaySeconds(value: number): void {
  if (!valid(value)) return;
  writePreference(delay, setDelay, value, next => backend().setAppString(KEY, String(next)), "hover preview delay");
}

/** Restore the device value; malformed stored values use the one-second default. */
export function initHoverPreviewSettings(): void {
  loadPreference(delay, setDelay, () => backend().getAppString(KEY, "1"), value => {
    const parsed = value.trim() === "" ? NaN : Number(value);
    return valid(parsed) ? parsed : 1;
  }, "hover preview delay");
}

/** No timer for values beyond the signed 32-bit timer range: browsers would
 * wrap them to an immediate preview. O(1), no I/O. */
export function hoverPreviewDelayMs(): number | null {
  const ms = delay() * 1000;
  return ms <= 2_147_483_647 ? ms : null;
}

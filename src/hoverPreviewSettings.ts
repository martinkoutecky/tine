// The preview settings (GH #824). The device-local mouse-hover delay, in
// seconds: touch hold has its own gesture delay, and reads and writes use the
// shared preference ownership/rollback. And the graph's preview switch,
// Logseq's `:ui/enable-tooltip?`, read from the served graph config and written
// through the shared graph-setting path.
import { createSignal } from "solid-js";
import { msg } from "./i18n";
import { backend } from "./backend";
import { graphMeta } from "./graphSession";
import { changeGraphSetting } from "./graphPreferences";
import { loadPreference, writePreference } from "./preferenceWrites";

const KEY = "hover_preview_delay_seconds";
const [delay, setDelay] = createSignal(1);
export const hoverPreviewDelaySeconds = delay;

const valid = (value: number) => Number.isFinite(value) && value >= 0;

/** Apply valid seconds and queue persistence. Failed writes roll back and toast. */
export function setHoverPreviewDelaySeconds(value: number): void {
  if (!valid(value)) return;
  writePreference(delay, setDelay, value, next => backend().setAppString(KEY, String(next)), msg("toast-save-hover-delay-failed"));
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

/** Whether page, tag and block-reference previews are on. OG's
 *  `:ui/enable-tooltip?` (`state/enable-tooltip?`, `src/main/frontend/state.cljs:688`
 *  at `6e7afa8eb`): on when absent, only an explicit false turns them off. */
export function previewsEnabled(): boolean {
  const meta = graphMeta();
  return meta ? meta.enable_tooltip : true;
}

export function changePreviewsEnabled(on: boolean): void {
  changeGraphSetting("enable_tooltip", on, (next) => backend().setEnableTooltip(next),
    msg("toast-reference-previews-save-failed"));
}

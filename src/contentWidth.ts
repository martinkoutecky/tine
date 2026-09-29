// Device-local reading-column widths (GH #382). Themes own the defaults; only an
// explicit user override is stored, in the device-settings store beside the other
// remembered preferences (`getAppString`/`setAppString`; "" means no override).
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { pushToast } from "./toasts";
import { advanceRevision, currentRevision, readOwned, revisionOwner, writeOwned } from "./owned";

export const DEFAULT_STANDARD_CONTENT_WIDTH = 810;
export const DEFAULT_CUSTOM_WIDE_CONTENT_WIDTH = 1280;
export const MIN_CONTENT_WIDTH = 320;
export const MAX_CONTENT_WIDTH = 8192;
export const CONTENT_WIDTH_SLIDER_MAX = 2400;

const STANDARD_KEY = "content_width_standard";
const WIDE_KEY = "content_width_wide";
const standardSlot = {};
const wideSlot = {};

function parseWidth(stored: string): number | null {
  if (stored === "") return null;
  const value = Number(stored);
  return Number.isFinite(value) && value >= MIN_CONTENT_WIDTH && value <= MAX_CONTENT_WIDTH
    ? Math.round(value)
    : null;
}

/** Clamp to the supported range and round. O(1). */
export function normalizeContentWidth(value: number): number {
  if (!Number.isFinite(value)) return MIN_CONTENT_WIDTH;
  return Math.min(MAX_CONTENT_WIDTH, Math.max(MIN_CONTENT_WIDTH, Math.round(value)));
}

export const [standardContentWidth, setStandardContentWidthSignal] = createSignal<number | null>(null);
export const [wideContentWidth, setWideContentWidthSignal] = createSignal<number | null>(null);

/** Apply only explicit user overrides; themes retain ownership of both defaults.
 * O(1), no I/O. */
export function applyContentWidths(): void {
  const root = document.documentElement;
  const standard = standardContentWidth();
  const wide = wideContentWidth();
  if (standard === null) root.style.removeProperty("--tine-main-content-max-width");
  else root.style.setProperty("--tine-main-content-max-width", `${standard}px`);
  if (wide === null) root.style.removeProperty("--tine-wide-content-max-width");
  else root.style.setProperty("--tine-wide-content-max-width", `${wide}px`);
}

function remember(key: string, slot: object, value: number | null): void {
  const revision = advanceRevision(slot);
  void writeOwned(revisionOwner(slot, revision), backend().setAppString(key, value === null ? "" : String(value)))
    .catch((error) => pushToast(`Could not remember the page width: ${String(error)}`, "error"));
}

/** Hydrate both overrides once at startup. A user's later change wins a delayed
 * read; a read failure keeps the theme defaults and reports an error. */
export async function initContentWidths(): Promise<void> {
  const load = async (key: string, slot: object, set: (value: number | null) => void) => {
    const owner = revisionOwner(slot, currentRevision(slot));
    try {
      const loaded = await readOwned(owner, backend().getAppString(key, ""));
      if (loaded.kind === "current") set(parseWidth(loaded.value));
    } catch (error) {
      pushToast(`Could not load the page width: ${String(error)}`, "error");
    }
  };
  await Promise.all([
    load(STANDARD_KEY, standardSlot, setStandardContentWidthSignal),
    load(WIDE_KEY, wideSlot, setWideContentWidthSignal),
  ]);
  applyContentWidths();
}

export function changeStandardContentWidth(value: number): void {
  if (!Number.isFinite(value)) return;
  const next = normalizeContentWidth(value);
  setStandardContentWidthSignal(next);
  remember(STANDARD_KEY, standardSlot, next);
  applyContentWidths();
}

export function resetStandardContentWidth(): void {
  setStandardContentWidthSignal(null);
  remember(STANDARD_KEY, standardSlot, null);
  applyContentWidths();
}

/** `null` fills the pane. */
export function changeWideContentWidth(value: number | null): void {
  if (value !== null && !Number.isFinite(value)) return;
  const next = value === null ? null : normalizeContentWidth(value);
  setWideContentWidthSignal(next);
  remember(WIDE_KEY, wideSlot, next);
  applyContentWidths();
}

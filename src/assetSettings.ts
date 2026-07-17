// Device-local asset-naming preference (persisted in tine-settings.json via the
// generic app_string backend for atomic, WebView-independent state). Read once at
// startup by initAssetSettings(); the signal drives both
// the insert-time tokenizer (media.ts assetFileName) and the Settings field.
//
// The value is a FORMAT TEMPLATE with `%`-tokens substituted per insert:
//   %assetname  the original file's stem, sanitized (empty for a clipboard paste)
//   %ext        the extension, lowercased, no dot (e.g. "png")
//   %yyyymmdd   local date, e.g. 20260628        %hhmmss  local time, e.g. 143002
//   %yyyy %yy %MM %dd %HH %mm %ss   granular local date/time parts (zero-padded)
// Any other characters are literal. See media.ts for substitution + the paste
// fallback (a paste has no %assetname, so an empty stem falls back to a stamp).

import { createSignal } from "solid-js";
import { backend } from "./backend";

const KEY = "asset_name_format";

/** Tine's default: the plain original filename (closest to OG for imported files).
 *  Collisions are still de-duplicated by the backend (`reserve_asset` → `_N`). */
export const DEFAULT_ASSET_NAME_FORMAT = "%assetname.%ext";
/** The previous Tine default — a sortable timestamp prefix — offered as a preset. */
export const STAMPED_ASSET_NAME_FORMAT = "%yyyymmdd-%hhmmss-%assetname.%ext";

const [fmt, setFmtSig] = createSignal(DEFAULT_ASSET_NAME_FORMAT);

/** Reactive: the asset-filename format template (read at insert time). */
export const assetNameFormat = fmt;

/** Set + persist the template. Blank reverts to the default. */
export function setAssetNameFormat(s: string): void {
  const v = s.trim() || DEFAULT_ASSET_NAME_FORMAT;
  setFmtSig(v);
  void backend().setAppString(KEY, v).catch(() => {});
}

/** Load the persisted template at startup (default = plain original name). */
export async function initAssetSettings(): Promise<void> {
  try {
    const v = await backend().getAppString(KEY, DEFAULT_ASSET_NAME_FORMAT);
    setFmtSig(v || DEFAULT_ASSET_NAME_FORMAT);
  } catch {
    /* keep the default */
  }
}

// Device-local security preference (persisted in tine-settings.json via the generic
// app_bool backend, so it survives a restart). OFF by default: when ON, raw-HTML
// `<img>` tags in notes may load images from absolute paths anywhere on this machine
// (see renderRawHtml + read_local_image / ADR 0019). A real permission — only enable
// for graphs you trust, since a synced/imported note isn't self-authored.

import { createSignal } from "solid-js";
import { backend } from "./backend";

const KEY = "allow_local_file_images";

const [allow, setAllowSig] = createSignal(false);

/** Reactive: raw-HTML `<img>` may load images from arbitrary local paths. */
export const allowLocalFileImages = allow;

export function setAllowLocalFileImages(on: boolean): void {
  setAllowSig(on);
  void backend().setAppBool(KEY, on).catch(() => {});
}

/** Load the persisted preference at startup. Default OFF. */
export async function initLocalFileSettings(): Promise<void> {
  try {
    setAllowSig(await backend().getAppBool(KEY, false));
  } catch {
    /* default off */
  }
}

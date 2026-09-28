// Device-local security preference (persisted in tine-settings.json via the generic
// app_bool backend, so it survives a restart). OFF by default: when ON, raw-HTML
// `<img>` tags in notes may load images from absolute paths anywhere on this machine
// (see renderRawHtml + read_local_image / ADR 0019). A real permission — only enable
// for graphs you trust, since a synced/imported note isn't self-authored.

import { createSignal } from "solid-js";
import { backend } from "./backend";
import { writePreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { pushToast } from "./toasts";

const KEY = "allow_local_file_images";

const [allow, setAllowSig] = createSignal(false);

/** Reactive: raw-HTML `<img>` may load images from arbitrary local paths. */
export const allowLocalFileImages = allow;

export function setAllowLocalFileImages(on: boolean): void {
  writePreference(allow, setAllowSig, on, (next) => backend().setAppBool(KEY, next), "local image access preference");
}

/** Load the persisted preference at startup. Default OFF. */
export async function initLocalFileSettings(): Promise<void> {
  const revision = preferenceRevision(allow);
  try {
    const value = await backend().getAppBool(KEY, false);
    if (preferenceReadCurrent(allow, revision)) { setAllowSig(value); seedPreference(allow); }
  } catch {
    pushToast("Could not load local image access preference.", "error");
  }
}

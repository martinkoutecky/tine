// Shared state + helpers for the mobile capture buttons (camera / voice memo).
// Recording is a single app-wide state (one mic), toggled from the focused
// editor's toolbar button; the block editor owns the actual start/stop + insert.
import { createSignal } from "solid-js";

/** True while a voice-memo recording is in progress (drives the mic button's
 *  stop icon + pulsing state in the toolbar). */
export const [isRecordingAudio, setRecordingAudio] = createSignal(false);

/** Decode a base64 string (as returned by the Android capture plugin) to bytes. */
export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

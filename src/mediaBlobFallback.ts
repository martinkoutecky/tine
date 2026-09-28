import { backend } from "./backend";
import { graphOwner, readOwned } from "./owned";

const AUDIO_MAX_BYTES = 64 * 1024 * 1024;
const VIDEO_MAX_BYTES = 128 * 1024 * 1024;
const TOTAL_MAX_BYTES = 128 * 1024 * 1024;

let retainedBytes = 0;
let queueTail: Promise<void> = Promise.resolve();

export type MediaBlobLease = {
  url: string;
  bytes: number;
  release: () => void;
};

function enqueue<T>(task: () => Promise<T>): Promise<T> {
  const result = queueTail.then(task, task);
  queueTail = result.then(() => {}, () => {});
  return result;
}

function abortError(): Error {
  return new DOMException("media fallback cancelled", "AbortError");
}

/** Serialize whole-file fallback reads for media rejected by the range-aware
 * protocol. Raw file bytes are capped at 64 MiB for audio, 128 MiB for video,
 * and 128 MiB retained across leases; the remaining raw-byte allowance is
 * passed to Rust before reading. This does not bound transient IPC/Blob copies.
 * Abort, stale graph ownership and budget exhaustion reject. Release the URL
 * lease when done; repeated release is harmless. */
export function acquireMediaBlobFallback(
  name: string,
  kind: "audio" | "video",
  mime: string,
  signal?: AbortSignal
): Promise<MediaBlobLease> {
  const owner = graphOwner(() => !signal?.aborted);
  return enqueue(async () => {
    if (signal?.aborted) throw abortError();
    if (!owner()) throw abortError();
    const perFileMax = kind === "audio" ? AUDIO_MAX_BYTES : VIDEO_MAX_BYTES;
    const remaining = TOTAL_MAX_BYTES - retainedBytes;
    if (remaining <= 0) throw new Error("media blob fallback budget exhausted");
    const result = await readOwned(owner, backend().readAsset(name, Math.min(perFileMax, remaining)));
    if (result.kind === "stale") throw abortError();
    const bytes = result.value;

    const size = bytes.byteLength;
    if (size > remaining) throw new Error("media blob fallback budget exceeded");
    const buffer = bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength
      ? bytes.buffer as ArrayBuffer
      : bytes.slice().buffer as ArrayBuffer;
    const url = URL.createObjectURL(new Blob([buffer], { type: mime }));
    retainedBytes += size;
    let released = false;
    return {
      url,
      bytes: size,
      release: () => {
        if (released) return;
        released = true;
        URL.revokeObjectURL(url);
        retainedBytes = Math.max(0, retainedBytes - size);
      },
    };
  });
}

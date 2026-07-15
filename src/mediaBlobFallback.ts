import { backend } from "./backend";

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

/**
 * Serialized, process-wide whole-file compatibility fallback for media that a
 * WebKit backend rejects through the range-aware custom protocol. The remaining
 * global allowance is passed to Rust before reading, so retained plus in-flight
 * encoded bytes never exceed the budget. Callers own and must release the lease.
 */
export function acquireMediaBlobFallback(
  name: string,
  kind: "audio" | "video",
  mime: string,
  signal?: AbortSignal
): Promise<MediaBlobLease> {
  return enqueue(async () => {
    if (signal?.aborted) throw abortError();
    const perFileMax = kind === "audio" ? AUDIO_MAX_BYTES : VIDEO_MAX_BYTES;
    const remaining = TOTAL_MAX_BYTES - retainedBytes;
    if (remaining <= 0) throw new Error("media blob fallback budget exhausted");
    const bytes = await backend().readAsset(name, Math.min(perFileMax, remaining));
    if (signal?.aborted) throw abortError();

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

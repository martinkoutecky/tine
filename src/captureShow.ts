/** Quick Capture's show lifetime (tine#985). Each show of the capture window
 * gets a fresh signal; hiding it or a new show aborts the old one, which
 * cancels a pending outside-file prompt and retires that show's late replies.
 * The native show generation is what a capture `file:` call carries, so native
 * refuses a click from an earlier show even when `capture-shown` was missed.
 * In graph windows `endCaptureShow` is never called and both are undefined. */
let controller: AbortController | null = null;
let nativeShow: number | undefined;

/** End the current show (hide, or a new `capture-shown`). O(1). */
export function endCaptureShow(): void {
  controller?.abort();
  controller = new AbortController();
}

/** The current capture show's signal; undefined outside Quick Capture. */
export function captureShowSignal(): AbortSignal | undefined {
  return controller?.signal;
}

/** Record the native show generation the capture binding was read for. */
export function noteCaptureShow(show: number): void {
  nativeShow = show;
}

export function captureShowGeneration(): number | undefined {
  return nativeShow;
}

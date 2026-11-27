/** Quick Capture's show lifetime (tine#985). Each show of the capture window
 * gets a fresh signal; hiding it or a new show aborts the old one, which
 * cancels a pending outside-file prompt and retires that show's late replies.
 * Which show a gesture belongs to is native's answer, never kept here: the
 * first `file:` call is admitted in native's current show and its Ask names
 * it, so a click in a show whose `capture-shown` was missed still prompts.
 * In graph windows `endCaptureShow` is never called and the signal is
 * undefined. */
let controller: AbortController | null = null;

/** End the current show (hide, or a new `capture-shown`). O(1). */
export function endCaptureShow(): void {
  controller?.abort();
  controller = new AbortController();
}

/** The current capture show's signal; undefined outside Quick Capture. */
export function captureShowSignal(): AbortSignal | undefined {
  return controller?.signal;
}

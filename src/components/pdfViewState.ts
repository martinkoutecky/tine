type ReadingState = { page: number; scale: number };

/** Local page tracking stays live. Publish scrolling only at idle, while zoom
 * and explicit navigation remain committed intents. No annotation writes. */
export function createPdfViewState(read: () => ReadingState,
  publish: (state: ReadingState) => void, current: () => boolean) {
  let baseline: ReadingState | null = null;
  let timer: number | undefined;

  function emit(page: number, scale: number) {
    if (!current() || !Number.isFinite(scale) || scale <= 0) return;
    if (baseline?.page === page && baseline?.scale === scale) return;
    baseline = { page, scale };
    publish(baseline);
  }
  function cancel() {
    window.clearTimeout(timer);
    timer = undefined;
  }
  function scroll() {
    cancel();
    timer = window.setTimeout(() => {
      timer = undefined;
      const state = read();
      emit(state.page, state.scale);
    }, 150);
  }
  function changed(page: number, scale: number) {
    if (timer === undefined || baseline?.scale !== scale) emit(page, scale);
    else scroll();
  }
  function flush() {
    if (timer !== undefined) {
      const state = read();
      emit(state.page, state.scale);
    }
    cancel();
  }
  return { publish: emit, changed, scroll, flush, cancel };
}

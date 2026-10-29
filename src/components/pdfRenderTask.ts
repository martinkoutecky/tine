import type { PDFPageProxy } from "pdfjs-dist";

// Restore pdf.js 4.10.38's _maybeCleanupAfterRender policy, removed in 5.x:
// display/api.js used a 10e6-byte image threshold and a 5000-ms idle grace.
// Keep the completed canvas; release only decoded images/operator lists. One
// page proxy is shared by splits and tiles, so their renders share this timer.
type State = { active: number; large: boolean; retired: boolean; timer?: ReturnType<typeof setTimeout> };
const states = new WeakMap<PDFPageProxy, State>();

function hasLargeImage(page: PDFPageProxy) {
  const objects = page.objs;
  if (!objects || typeof objects[Symbol.iterator] !== "function") return false;
  for (const [, value] of objects) {
    const image = value as { dataLen?: number; data?: ArrayBufferView; bitmap?: { width: number; height: number } } | null;
    if (!image) continue;
    const bytes = image.dataLen ?? image.data?.byteLength
      ?? (image.bitmap ? image.bitmap.width * image.bitmap.height * 4 : 0);
    if (bytes > 10_000_000) return true;
  }
  return false;
}

function cancelTimer(state: State) {
  if (state.timer !== undefined) clearTimeout(state.timer);
  state.timer = undefined;
}

export function renderPdfPage(page: PDFPageProxy, parameters: Parameters<PDFPageProxy["render"]>[0]) {
  let state = states.get(page);
  if (!state) states.set(page, state = { active: 0, large: false, retired: false });
  const current = state;
  cancelTimer(current);
  current.retired = false;
  current.active++;
  const complete = () => {
    current.active--;
    if (current.retired) return;
    current.large ||= hasLargeImage(page);
    if (current.active || !current.large) return;
    current.timer = setTimeout(() => {
      current.timer = undefined;
      page.cleanup(); // pdf.js itself defers while an operator list is pending.
    }, 5000);
  };
  try {
    const task = page.render(parameters);
    void task.promise.then(complete, complete);
    return task;
  } catch (error) {
    complete();
    throw error;
  }
}

export function releasePdfPage(page: PDFPageProxy) {
  const state = states.get(page);
  if (state) { state.retired = true; cancelTimer(state); states.delete(page); }
  return page.cleanup();
}

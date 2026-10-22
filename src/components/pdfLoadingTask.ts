import { discardPdfDocument } from "./pdfViewerPrimitives";

/** The loading task owns the worker until its proxy is accepted. Cancellation
 * clears ownership before destroy: rejection and teardown may race. */
export function createPdfLoadingTask() {
  let pending: { destroy(): Promise<void> } | null = null;
  function cancel() {
    const task = pending;
    pending = null;
    if (task) discardPdfDocument(task);
  }
  return {
    cancel,
    async load<T>(task: { promise: Promise<T>; destroy(): Promise<void> }, current: () => boolean, accept: (proxy: T) => void): Promise<void> {
      pending = task;
      try {
        const proxy = await task.promise;
        if (pending !== task || !current()) { cancel(); return; }
        pending = null; // The accepted document proxy now owns destruction.
        // Transfer synchronously, so teardown before the caller's next
        // microtask can already see and retire the proxy.
        accept(proxy);
      } catch (error) {
        const active = pending === task && current();
        cancel();
        if (active) throw error;
      }
    },
  };
}

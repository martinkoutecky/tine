import { createSignal, type Accessor } from "solid-js";

export interface PdfNavigationIntent {
  serial: number;
  page?: number;
  highlightId?: string;
}

const intents = new Map<string, ReturnType<typeof createSignal<PdfNavigationIntent | null>>>();
let serial = 0;

function channel(viewId: string) {
  let signal = intents.get(viewId);
  if (!signal) {
    signal = createSignal<PdfNavigationIntent | null>(null);
    intents.set(viewId, signal);
  }
  return signal;
}

/** Read the latest explicit navigation request for one reader view. A route
 * page/scale update never changes this signal. Cost O(1). */
export function pdfNavigationIntent(viewId: string): Accessor<PdfNavigationIntent | null> {
  return channel(viewId)[0];
}

/** Request navigation within a mounted reader without replacing its route. */
export function publishPdfNavigationIntent(
  viewId: string,
  intent: Omit<PdfNavigationIntent, "serial">,
): PdfNavigationIntent {
  const next = { ...intent, serial: ++serial };
  channel(viewId)[1](next);
  return next;
}

/** Discard a closed view's transient request channel. */
export function retirePdfNavigationIntent(viewId: string): void {
  intents.delete(viewId);
}

/** Clear transient request channels between isolated tests or graph sessions. */
export function resetPdfNavigationForTest(): void {
  intents.clear();
  serial = 0;
}

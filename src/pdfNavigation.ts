import { createSignal, type Accessor } from "solid-js";

export interface PdfNavigationIntent {
  serial: number;
  page?: number;
  highlightId?: string;
}

const intents = new Map<string, ReturnType<typeof createSignal<PdfNavigationIntent | null>>>();
let serial = 0;

function channel(viewId: string) {
  let value = intents.get(viewId);
  if (!value) {
    value = createSignal<PdfNavigationIntent | null>(null);
    intents.set(viewId, value);
  }
  return value;
}

export function pdfNavigationIntent(viewId: string): Accessor<PdfNavigationIntent | null> {
  return channel(viewId)[0];
}

export function publishPdfNavigationIntent(
  viewId: string,
  intent: Omit<PdfNavigationIntent, "serial">,
): PdfNavigationIntent {
  const next = { ...intent, serial: ++serial };
  channel(viewId)[1](next);
  return next;
}

export function retirePdfNavigationIntent(viewId: string): void {
  intents.delete(viewId);
}

export function resetPdfNavigationForTest(): void {
  intents.clear();
  serial = 0;
}

import { bindingCurrent, captureBinding, clearOnBindingInvalidated } from "./binding";

export type ExternalPdfRead = () => Promise<Uint8Array>;
const sources = new Map<string, { filename: string; read: ExternalPdfRead }>();
clearOnBindingInvalidated(() => sources.clear());

/** Called only by the host's explicit link-open route. A saved session never
 * contains this closure. No file I/O until the new reader consumes its source. */
export function authorizeExternalPdf(viewId: string, filename: string, read: ExternalPdfRead): void {
  const binding = captureBinding();
  sources.set(viewId, { filename, read: () => bindingCurrent(binding)
    ? read() : Promise.reject(new Error("stale PDF graph binding")) });
}

/** Consume once: mounting a restored route or revisiting it after its document
 * was released cannot silently read an outside file again. */
export function takeExternalPdfSource(viewId: string, filename: string): ExternalPdfRead | undefined {
  const source = sources.get(viewId);
  sources.delete(viewId);
  return source?.filename === filename ? source.read : undefined;
}

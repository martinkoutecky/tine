// Measurement hook for the PDF viewer perf harness (scripts/perf-pdf, part of
// the pre-stable perf gate; D-20261021-PV2). A no-op unless the harness
// installs globalThis.__tinePdfProbe; it then receives raster, tile, text-layer
// and zoom lifecycle events. Never put user content in an event.
export type PdfProbeEvent = { type: string; page: number; [key: string]: unknown };
type Probe = (event: PdfProbeEvent & { t: number }) => void;

export function pdfProbe(event: PdfProbeEvent): void {
  const probe = (globalThis as { __tinePdfProbe?: Probe }).__tinePdfProbe;
  if (probe) probe({ ...event, t: performance.now() });
}

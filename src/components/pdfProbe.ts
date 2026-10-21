// MEASUREMENT INSTRUMENTATION — evidence branch batch/og-pdf-perf-review only.
// Never integrate. A no-op unless a perf harness installs
// globalThis.__tinePdfProbe; it then receives raster/tile/text lifecycle events.
export type PdfProbeEvent = { type: string; page: number; [key: string]: unknown };
type Probe = (event: PdfProbeEvent & { t: number }) => void;

export function pdfProbe(event: PdfProbeEvent): void {
  const probe = (globalThis as { __tinePdfProbe?: Probe }).__tinePdfProbe;
  if (probe) probe({ ...event, t: performance.now() });
}

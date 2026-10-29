/** pdf.js 5's TextLayer CSS expects these variables on a stock .page ancestor.
 * Tine owns its wrappers, so pin them on the text layer itself. The layer keeps
 * this scale while a zoom stretches it until its next owned update. */
export function pinPdfTextScale(layer: HTMLElement, scale: number, userUnit: number) {
  layer.style.setProperty("--scale-factor", String(scale));
  layer.style.setProperty("--total-scale-factor", String(scale * userUnit));
  layer.style.setProperty("--scale-round-x", "1px");
  layer.style.setProperty("--scale-round-y", "1px");
  layer.dataset.scale = String(scale);
  layer.style.transform = "";
}

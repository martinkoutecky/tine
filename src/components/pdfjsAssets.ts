/** pdf.js 5 loads image decoders, fonts and colour profiles lazily. All paths
 * stay inside this application's bundle, including on Tauri's custom scheme.
 * pdf.js selects its fetching factory from the supplied URL scheme. */
export function pdfjsAssetOptions() {
  const root = new URL("./pdfjs/", document.baseURI).href;
  return {
    cMapUrl: `${root}cmaps/`,
    standardFontDataUrl: `${root}standard_fonts/`,
    wasmUrl: `${root}wasm/`,
    iccUrl: `${root}iccs/`,
  };
}

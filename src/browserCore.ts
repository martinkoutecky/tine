import { base64ToBytes } from "./render/wasmBytes";
import { BrowserCoreLoadError } from "./browserCoreError";

// The one module that imports the core chunk. Its importers are the publication
// worker and the browser mock's graph; vite.config.ts `browserCoreAssets` pins it.
let core: ReturnType<typeof initializeBrowserCore> | undefined;

async function initializeBrowserCore() {
  try {
    const { wasm, WASM_B64, LSDOC_TAG } = await import("./render/wasm-core/runtime");
    await wasm.default({ module_or_path: base64ToBytes(WASM_B64) });
    if (wasm.lsdoc_tag() !== LSDOC_TAG) throw new Error("lsdoc-wasm core tag mismatch");
    return wasm;
  } catch (error) {
    throw new BrowserCoreLoadError(error);
  }
}

/** Load the browser graph core once. See ADR 0006 for the packaging contract. */
export function loadBrowserCore() {
  return (core ??= initializeBrowserCore());
}

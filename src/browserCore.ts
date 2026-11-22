import { base64ToBytes } from "./render/parse";

let core: ReturnType<typeof initializeBrowserCore> | undefined;

/** The browser could not initialize the graph engine. */
export class BrowserCoreLoadError extends Error {
  constructor(cause: unknown) {
    super("Could not initialize the browser graph engine", { cause });
  }
}

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

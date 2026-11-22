import { base64ToBytes } from "./render/parse";

let core: ReturnType<typeof initializeBrowserCore> | undefined;

async function initializeBrowserCore() {
  const { wasm, WASM_B64, LSDOC_TAG } = await import("./render/wasm-core/runtime");
  await wasm.default({ module_or_path: base64ToBytes(WASM_B64) });
  if (wasm.lsdoc_tag() !== LSDOC_TAG) throw new Error("lsdoc-wasm core tag mismatch");
  return wasm;
}

/** Load the browser graph core once. See ADR 0006 for the packaging contract. */
export function loadBrowserCore() {
  return (core ??= initializeBrowserCore());
}

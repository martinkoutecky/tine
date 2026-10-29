import { expect, it } from "vitest";
import { matchOpenjpegImports } from "../scripts/lib/pdfjs-openjpeg-abi.mjs";

// A module that exports its imported function as run(). This checks real WASM
// linking and execution, including preservation of the caller's source bytes.
const moduleBytes = new Uint8Array([
  0, 97, 115, 109, 1, 0, 0, 0,
  1, 5, 1, 96, 0, 1, 127,
  2, 7, 1, 1, 97, 1, 120, 0, 0,
  7, 7, 1, 3, 114, 117, 110, 0, 0,
]);
const glue = 'var wasmImports={x:_value};function assignWasmExports(wasmExports){_run=wasmExports["run"]}';
const worker = 'async function OpenJPEG() { var wasmImports = { y: _value }; function assignWasmExports(wasmExports) { _run = wasmExports["run"]; } } /* harmony default export */ const openjpeg = (OpenJPEG);';

it("links each generated import to the worker's semantic function", () => {
  const output = matchOpenjpegImports(moduleBytes, glue, worker);
  const instance = new WebAssembly.Instance(new WebAssembly.Module(output), { a: { y: () => 42 } });
  expect((instance.exports.run as () => number)()).toBe(42);
  expect(WebAssembly.Module.imports(new WebAssembly.Module(moduleBytes))[0].name).toBe("x");
});

it("refuses a worker whose decoder interface changed", () => {
  expect(() => matchOpenjpegImports(moduleBytes, glue, worker.replace("_value", "_other"))).toThrow(/import changed/);
  expect(() => matchOpenjpegImports(moduleBytes, glue, worker.replace('wasmExports["run"]', 'wasmExports["other"]'))).toThrow(/exports changed/);
  expect(() => matchOpenjpegImports(moduleBytes, glue, worker.replace("OpenJPEG", "Other"))).toThrow(/boundary changed/);
});

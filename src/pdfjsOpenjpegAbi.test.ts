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

it("loads Emscripten 3.1 exports under the names the unchanged worker uses", () => {
  const legacyWasm = new Uint8Array([
    ...moduleBytes.subarray(0, 24),
    4, 4, 1, 112, 0, 0, // Table.
    7, 13, 3,
    1, 118, 1, 0, // v: table.
    1, 116, 0, 0, // t: run.
    1, 117, 0, 0, // u: constructor.
  ]);
  const legacyGlue = 'var wasmImports={x:_value};function initRuntime(wasmExports){wasmExports["u"]()}_run=wasmExports["t"];wasmTable=wasmExports["v"];';
  const workerWithTable = worker.replace("var wasmImports", 'function initRuntime() { wasmExports["s"](); } var wasmImports')
    .replace('_run = wasmExports["run"];', '_run = wasmExports["run"]; __indirect_function_table = wasmExports["__indirect_function_table"];');

  const output = matchOpenjpegImports(legacyWasm, legacyGlue, workerWithTable, { legacyExports: true });
  const instance = new WebAssembly.Instance(new WebAssembly.Module(output), { a: { y: () => 42 } });
  expect((instance.exports.run as () => number)()).toBe(42);
  expect((instance.exports.s as () => number)()).toBe(42);
  expect(instance.exports.__indirect_function_table).toBeInstanceOf(WebAssembly.Table);
  expect(() => matchOpenjpegImports(legacyWasm, legacyGlue.replace('wasmExports["v"]', 'wasmExports["t"]'),
    workerWithTable, { legacyExports: true })).toThrow(/exports changed/);
});

it("loads Debian's decoder when it omits unused worker runtime hooks", () => {
  const legacyWasm = new Uint8Array([
    ...moduleBytes.subarray(0, 24),
    7, 9, 2, 1, 116, 0, 0, 1, 117, 0, 0, // t: run, u: constructor.
  ]);
  const legacyGlue = 'var wasmImports={x:_value};function initRuntime(wasmExports){wasmExports["u"]()}_run=wasmExports["t"];';
  const workerWithRuntime = worker.replace("var wasmImports", 'function initRuntime() { wasmExports["s"](); } var wasmImports')
    .replace("y: _value", "y: _value, a: __abort_js, b: __emscripten_runtime_keepalive_clear, c: __setitimer_js, d: _proc_exit")
    .replace('_run = wasmExports["run"];', '_run = wasmExports["run"]; __emscripten_timeout = wasmExports["w"];');

  const output = matchOpenjpegImports(legacyWasm, legacyGlue, workerWithRuntime, { legacyExports: true });
  const instance = new WebAssembly.Instance(new WebAssembly.Module(output), { a: { y: () => 42 } });
  expect((instance.exports.run as () => number)()).toBe(42);
  expect(() => matchOpenjpegImports(legacyWasm, legacyGlue.replace("x:_value", "x:_value,z:__setitimer_js"),
    workerWithRuntime, { legacyExports: true })).toThrow(/exports changed/);
  expect(() => matchOpenjpegImports(legacyWasm, legacyGlue, workerWithRuntime.replace("y: _value", "y: _value, z: _copy_pixels_1"),
    { legacyExports: true })).toThrow(/imports changed/);
});

it("refuses a worker whose decoder interface changed", () => {
  expect(() => matchOpenjpegImports(moduleBytes, glue, worker.replace("_value", "_other"))).toThrow(/import changed/);
  expect(() => matchOpenjpegImports(moduleBytes, glue, worker.replace('wasmExports["run"]', 'wasmExports["other"]'))).toThrow(/exports changed/);
  expect(() => matchOpenjpegImports(moduleBytes, glue, worker.replace("OpenJPEG", "Other"))).toThrow(/boundary changed/);
});

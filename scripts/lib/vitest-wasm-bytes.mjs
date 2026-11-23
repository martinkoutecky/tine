// Test-only transport for the browser core's inlined wasm (ADR 0081). The
// generated `src/render/wasm-core/lsdoc_wasm_bytes.ts` is a 6 MB base64 string;
// served through vitest's module server it is transformed, given an inline
// source map and sent to every test file that loads the core, and that single
// server thread then slows every other file. This plugin serves the module as a
// stub that reads the same generated file from disk in the test process: the
// same bytes, the same tag, the same core. Builds never use it.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const CORE_BYTES = fileURLToPath(new URL("../../src/render/wasm-core/lsdoc_wasm_bytes.ts", import.meta.url));

/** Read `export const <name> = "<value>";` from the generated module's text.
 * Inlined into the stub, which runs in the test environment (jsdom included). */
function generatedConstant(source, name) {
  const start = source.indexOf(`export const ${name} = "`);
  if (start < 0) throw new Error(`lsdoc_wasm_bytes.ts has no ${name}`);
  const from = start + `export const ${name} = "`.length;
  return source.slice(from, source.indexOf('"', from));
}

export function vitestWasmBytes() {
  return {
    name: "tine-vitest-wasm-bytes",
    enforce: "pre",
    load(id) {
      if (id.split("?")[0] !== CORE_BYTES) return null;
      return [
        `import { readFileSync } from "node:fs";`,
        `${generatedConstant}`,
        `const source = readFileSync(${JSON.stringify(CORE_BYTES)}, "utf8");`,
        `export const LSDOC_TAG = generatedConstant(source, "LSDOC_TAG");`,
        `export const WASM_B64 = generatedConstant(source, "WASM_B64");`,
      ].join("\n");
    },
  };
}

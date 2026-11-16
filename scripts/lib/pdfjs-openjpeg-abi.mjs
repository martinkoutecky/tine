// Emscripten's minified import names depend on compiler output. Keep pdf.js's
// worker unchanged by translating only those names, using both generated glue
// tables as the authority. Never guess an import from its position.
function importNames(glue) {
  const table = /var wasmImports\s*=\s*\{([^}]+)\}/.exec(glue)?.[1];
  if (!table) throw new Error("OpenJPEG import table missing");
  const names = new Map();
  for (const entry of table.split(",")) {
    const pair = /^\s*(\w+)\s*:\s*(\w+)\s*$/.exec(entry);
    if (!pair || names.has(pair[2])) throw new Error("Unexpected OpenJPEG import table");
    names.set(pair[2], pair[1]);
  }
  return names;
}

function exportsTable(glue) {
  const body = /function assignWasmExports\([^)]*\)\s*\{([^}]+)\}/.exec(glue)?.[1];
  if (!body) throw new Error("OpenJPEG export table missing");
  return body.replace(/\s/g, "").replace(/;$/, "");
}

export function matchOpenjpegImports(input, generatedGlue, bundledWorker) {
  const start = bundledWorker.indexOf("async function OpenJPEG(");
  const end = bundledWorker.indexOf("/* harmony default export */ const openjpeg", start);
  if (start < 0 || end < 0) throw new Error("Pinned OpenJPEG worker boundary changed");
  const worker = bundledWorker.slice(start, end);
  if (exportsTable(generatedGlue) !== exportsTable(worker)) throw new Error("OpenJPEG exports changed");
  const generated = importNames(generatedGlue), target = importNames(worker);
  if (generated.size !== target.size) throw new Error("OpenJPEG imports changed");
  const rename = new Map();
  for (const [symbol, name] of generated) {
    const other = target.get(symbol);
    if (!other || name.length !== 1 || other.length !== 1) throw new Error(`OpenJPEG import changed: ${symbol}`);
    rename.set(name, other);
  }
  const bytes = new Uint8Array(input), decoder = new TextDecoder();
  let p = 8, seen = 0;
  const number = () => {
    let n = 0, shift = 0, b;
    do {
      if (p >= bytes.length || shift > 28) throw new Error("Invalid OpenJPEG WASM");
      b = bytes[p++]; n |= (b & 127) << shift; shift += 7;
    } while (b & 128);
    return n;
  };
  const string = () => {
    const size = number(), offset = p; p += size;
    return { value: decoder.decode(bytes.subarray(offset, p)), offset, size };
  };
  while (p < bytes.length) {
    const section = bytes[p++], size = number(), end = p + size;
    if (end > bytes.length) throw new Error("Invalid OpenJPEG WASM section");
    if (section === 2) {
      const count = number();
      for (let i = 0; i < count; i++) {
        const module = string(), name = string(), kind = bytes[p++];
        if (module.value !== "a" || kind !== 0 || name.size !== 1 || !rename.has(name.value)) {
          throw new Error("Unexpected OpenJPEG WASM import");
        }
        number(); // Function type index; no code, types or memory limits change.
        bytes[name.offset] = rename.get(name.value).charCodeAt(0);
        seen++;
      }
      if (p !== end) throw new Error("Invalid OpenJPEG WASM imports");
    }
    p = end;
  }
  if (seen !== rename.size || !WebAssembly.validate(bytes)) throw new Error("Invalid OpenJPEG WASM interface");
  return bytes;
}

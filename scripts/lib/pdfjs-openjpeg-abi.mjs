// Use both glue files to match the decoder's minified symbols to pdf.js's worker.
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
  return body;
}

function exportBindings(glue, assignments = glue) {
  const ctor = /function initRuntime\([^)]*\)\s*\{[^}]*?wasmExports\["(\w+)"\]\(\)/.exec(glue)?.[1];
  if (!ctor) throw new Error("OpenJPEG constructor export missing");
  const bindings = new Map([["__wasm_call_ctors", ctor]]);
  for (const [, binding, name] of assignments.matchAll(/\b(\w+)\s*=\s*(?:Module\["\1"\]\s*=\s*)?wasmExports\["(\w+)"\]/g)) {
    const symbol = binding === "wasmTable" ? "__indirect_function_table" : binding;
    if (bindings.has(symbol)) throw new Error("OpenJPEG exports changed");
    bindings.set(symbol, name);
  }
  return bindings;
}

function legacyExportRenames(glue, worker, workerTable, usesTimer) {
  const generated = exportBindings(glue), target = exportBindings(worker, workerTable);
  if (!usesTimer && !generated.has("__emscripten_timeout")) target.delete("__emscripten_timeout");
  const renames = new Map();
  if (generated.size !== target.size || new Set(target.values()).size !== target.size) throw new Error("OpenJPEG exports changed");
  for (const [symbol, name] of generated) {
    const other = target.get(symbol);
    if (!other || renames.has(name)) throw new Error("OpenJPEG exports changed");
    const kind = symbol === "wasmMemory" ? 2 : symbol === "__indirect_function_table" ? 1 : 0;
    renames.set(name, { name: other, kind });
  }
  return renames;
}

function encodeNumber(n) {
  const bytes = [];
  do {
    const digit = n & 127;
    n >>>= 7;
    bytes.push(n ? digit | 128 : digit);
  } while (n);
  return bytes;
}

export function matchOpenjpegImports(input, generatedGlue, bundledWorker, { legacyExports = false } = {}) {
  const start = bundledWorker.indexOf("async function OpenJPEG(");
  const end = bundledWorker.indexOf("/* harmony default export */ const openjpeg", start);
  if (start < 0 || end < 0) throw new Error("Pinned OpenJPEG worker boundary changed");
  const worker = bundledWorker.slice(start, end);
  const generated = importNames(generatedGlue), target = importNames(worker);
  const workerTable = exportsTable(worker);
  const exportRenames = legacyExports
    ? legacyExportRenames(generatedGlue, worker, workerTable, generated.has("__setitimer_js")) : null;
  const normalize = table => table.replace(/\s/g, "").replace(/;$/, "");
  if (!legacyExports && normalize(exportsTable(generatedGlue)) !== normalize(workerTable)) {
    throw new Error("OpenJPEG exports changed");
  }
  if (legacyExports) {
    // Debian's build drops these unused runtime hooks. The worker can supply them unconditionally.
    for (const symbol of ["__abort_js", "__emscripten_runtime_keepalive_clear", "__setitimer_js", "_proc_exit"]) {
      if (!generated.has(symbol)) target.delete(symbol);
    }
  }
  if (generated.size !== target.size) throw new Error("OpenJPEG imports changed");
  const rename = new Map();
  for (const [symbol, name] of generated) {
    const other = target.get(symbol);
    if (!other || name.length !== 1 || other.length !== 1) throw new Error(`OpenJPEG import changed: ${symbol}`);
    rename.set(name, other);
  }
  const bytes = new Uint8Array(input), decoder = new TextDecoder();
  let p = 8, seen = 0, exportSection;
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
    const start = p;
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
    if (section === 7 && exportRenames) {
      const count = number(), payload = encodeNumber(count);
      for (let i = 0; i < count; i++) {
        const { value } = string(), entry = p;
        const kind = bytes[p++];
        number(); // Index into the corresponding WASM section.
        const binding = exportRenames.get(value);
        if (!binding || binding.kind !== kind) throw new Error("Unexpected OpenJPEG WASM export");
        payload.push(...encodeNumber(binding.name.length), ...Buffer.from(binding.name), ...bytes.subarray(entry, p));
        exportRenames.delete(value);
      }
      if (p !== end || exportRenames.size) throw new Error("Invalid OpenJPEG WASM exports");
      exportSection = { start, end, payload };
    }
    p = end;
  }
  const output = exportSection ? Buffer.concat([
    bytes.subarray(0, exportSection.start),
    Buffer.from([7, ...encodeNumber(exportSection.payload.length), ...exportSection.payload]),
    bytes.subarray(exportSection.end),
  ]) : bytes;
  if (seen !== rename.size || exportRenames?.size || !WebAssembly.validate(output)) throw new Error("Invalid OpenJPEG WASM interface");
  return output;
}

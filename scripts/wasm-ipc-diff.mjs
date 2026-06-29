#!/usr/bin/env node
// Diff oracle: prove the vendored wasm parser (src/render/wasm) and the Rust index
// parser (tine_core::render::parse_block) produce IDENTICAL ASTs over a real-graph
// corpus — i.e. the duplicated bullet re-prepend (plan §3/§7C) has not drifted.
//
// For each {raw, format} in a block-raws.json:
//   wasm side: load the vendored base64 wasm + glue, call parse_block_json.
//   rust side: cargo run --example parse-corpus (tine_core::render::parse_block).
// Deep-compare per block (both canonicalized through JS JSON.stringify).
//
// Run with cargo on PATH (source scripts/env.sh first):
//   node scripts/wasm-ipc-diff.mjs <block-raws.json> [<block-raws.json> ...]

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const corpora = process.argv.slice(2);
if (corpora.length === 0) {
  console.error("usage: node scripts/wasm-ipc-diff.mjs <block-raws.json> ...");
  process.exit(2);
}

// --- load the vendored wasm exactly as the app does (base64 bytes, no fetch) ---
const bytesTs = readFileSync(join(root, "src/render/wasm/lsdoc_wasm_bytes.ts"), "utf8");
const b64 = bytesTs.match(/WASM_B64\s*=\s*"([^"]*)"/)?.[1];
if (!b64) throw new Error("could not read WASM_B64 from lsdoc_wasm_bytes.ts");
const wasmBytes = Buffer.from(b64, "base64");
const glue = await import(join(root, "src/render/wasm/lsdoc_wasm.js"));
await glue.default({ module_or_path: wasmBytes });

let total = 0;
let mismatches = 0;
for (const corpus of corpora) {
  const records = JSON.parse(readFileSync(corpus, "utf8"));
  // rust side: one process per corpus.
  const rustJson = execFileSync(
    "cargo",
    ["run", "-q", "-p", "tine-core", "--example", "parse-corpus", "--", corpus],
    { cwd: root, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
  );
  const rustAsts = JSON.parse(rustJson);

  let bad = 0;
  records.forEach((r, i) => {
    total++;
    const isOrg = r.format === "org";
    const wasmObj = JSON.parse(glue.parse_block_json(r.raw ?? "", isOrg));
    const a = JSON.stringify(wasmObj);
    const b = JSON.stringify(rustAsts[i]);
    if (a !== b) {
      bad++;
      mismatches++;
      console.error(`\nMISMATCH ${corpus}#${i} (format=${r.format}):`);
      console.error(`  raw : ${JSON.stringify(r.raw)}`);
      console.error(`  wasm: ${a}`);
      console.error(`  rust: ${b}`);
    }
  });
  console.log(`${corpus}: ${records.length} blocks, ${bad} mismatch${bad === 1 ? "" : "es"}`);
}

console.log(`\nTOTAL: ${total} blocks, ${mismatches} mismatch${mismatches === 1 ? "" : "es"}`);
process.exit(mismatches === 0 ? 0 : 1);

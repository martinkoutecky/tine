#!/usr/bin/env node
// Shipping guard for stale or oversized vendored Wasm.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { assertWithinCeiling, readLsdocTag, root, wasmArtifacts } from "./wasm-artifacts.mjs";

const expected = readLsdocTag();
for (const { dir, maxBytes } of wasmArtifacts) {
  const rel = `${dir}/lsdoc_wasm_bytes.ts`;
  const source = readFileSync(join(root, rel), "utf8");
  const tag = source.match(/^export const LSDOC_TAG = "([^"]+)";$/m)?.[1];
  if (tag !== expected) {
    throw new Error(`${rel}: expected lsdoc ${expected}, found ${tag ?? "no tag"}; run npm run build:wasm`);
  }
  const base64 = source.match(/^export const WASM_B64 = "([A-Za-z0-9+/]+={0,2})";$/m)?.[1];
  if (!base64 || base64.length % 4 !== 0) throw new Error(`${rel}: missing or invalid Wasm base64`);
  const bytes = Buffer.byteLength(base64, "base64");
  assertWithinCeiling(rel, bytes, maxBytes);
  console.log(`wasm pin OK: ${dir}, lsdoc ${tag}, ${bytes}/${maxBytes} raw bytes`);
}

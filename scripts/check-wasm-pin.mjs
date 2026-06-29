#!/usr/bin/env node
// Stale-vendored-wasm SHIPPING guard (audit finding C5).
//
// `build:wasm` only checks the pins when you regenerate; nothing stopped a build
// that bumped the lsdoc tag in Cargo but forgot to rerun `build:wasm` from shipping
// the OLD parser. This runs as part of `npm run build` (which CI invokes), and FAILS
// the build unless all three agree:
//   - crates/tine-core/Cargo.toml   lsdoc tag (the source of truth)
//   - crates/lsdoc-wasm/Cargo.toml  lsdoc tag
//   - src/render/wasm/lsdoc_wasm_bytes.ts  LSDOC_TAG (what the vendored bytes were built from)

import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function cargoTag(rel) {
  const m = readFileSync(join(root, rel), "utf8").match(/lsdoc\s*=\s*\{[^}]*\btag\s*=\s*"([^"]+)"/);
  if (!m) throw new Error(`could not find lsdoc tag in ${rel}`);
  return m[1];
}
function vendoredTag() {
  const m = readFileSync(join(root, "src/render/wasm/lsdoc_wasm_bytes.ts"), "utf8").match(
    /LSDOC_TAG\s*=\s*"([^"]+)"/,
  );
  if (!m) throw new Error("could not find LSDOC_TAG in src/render/wasm/lsdoc_wasm_bytes.ts");
  return m[1];
}

const core = cargoTag("crates/tine-core/Cargo.toml");
const wrap = cargoTag("crates/lsdoc-wasm/Cargo.toml");
const vendored = vendoredTag();

if (core !== wrap || core !== vendored) {
  console.error(
    `\n  Stale vendored wasm parser (build aborted):\n` +
      `    crates/tine-core/Cargo.toml   -> ${core}\n` +
      `    crates/lsdoc-wasm/Cargo.toml  -> ${wrap}\n` +
      `    src/render/wasm (vendored)    -> ${vendored}\n` +
      `  Align the Cargo pins and run \`npm run build:wasm\` to regenerate.\n`,
  );
  process.exit(1);
}
console.log(`wasm pin OK: lsdoc ${core} (Cargo ×2 == vendored bytes)`);

import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const root = resolve(fileURLToPath(new URL("..", import.meta.url)));

export const wasmArtifacts = [
  { dir: "src/render/wasm", feature: null, maxBytes: 1_551_997 },
  { dir: "src/render/wasm-core", feature: "core", maxBytes: 4_631_781 },
];

export function readLsdocTag() {
  const manifests = ["tine-core", "tine-outline", "lsdoc-wasm", "tine-graph-features"];
  const tags = manifests.map((name) => {
    const rel = `crates/${name}/Cargo.toml`;
    const pin = readFileSync(join(root, rel), "utf8").match(/lsdoc\s*=\s*\{[^}]*\btag\s*=\s*"([^"]+)"/);
    if (!pin) throw new Error(`could not find lsdoc tag in ${rel}`);
    return pin[1];
  });
  if (!tags.every((tag) => tag === tags[0])) {
    throw new Error(`lsdoc pin mismatch: ${manifests.map((name, i) => `${name}=${tags[i]}`).join(", ")}`);
  }
  return tags[0];
}

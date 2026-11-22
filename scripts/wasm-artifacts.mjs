import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const root = resolve(fileURLToPath(new URL("..", import.meta.url)));

// Fixed size ceilings, about 0.5% above the committed modules (1,551,997 and
// 4,631,781 bytes on 2026-11-22): wasm-opt's output differs by a few hundred
// bytes between hosts (CI, F-Droid), and every build path applies the same
// ceiling. Growth past one is a decision, not an automatic raise.
export const wasmArtifacts = [
  { dir: "src/render/wasm", feature: null, maxBytes: 1_560_000 },
  { dir: "src/render/wasm-core", feature: "core", maxBytes: 4_655_000 },
];

/** The one size check for a vendored module, used before regeneration writes
 *  and by the shipping guard. */
export function assertWithinCeiling(dir, bytes, maxBytes) {
  if (bytes > maxBytes) throw new Error(`${dir}: ${bytes} Wasm bytes exceed the ${maxBytes}-byte ceiling`);
}

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

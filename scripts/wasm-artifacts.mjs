import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const root = resolve(fileURLToPath(new URL("..", import.meta.url)));

// Fixed size ceilings, about 0.5% above the committed modules (1,551,997 and
// 4,631,781 bytes on 2026-11-22): wasm-opt's output differs by a few hundred
// bytes between hosts (CI, F-Droid), and every build path applies the same
// ceiling. Growth past one is a decision, not an automatic raise.
// 2026-11-23, browser core slice 5a: the core ceiling moved to 4,768,000 for the
// mock's edit API (measured: submit through the one page serializer 54.7k,
// file replace 36.1k, five shared answers 20.3k; tine-agents
// subagent-tasks/notes/browser-core-5a-size.md). No trim fits the old ceiling
// without a second serializer.
// 2026-11-24, local-target owner: the core ceiling moved to 4,794,000 (about
// 0.5% above the measured 4,770,096) for the one owner of link targets
// (crates/tine-core/src/local_target.rs, about 12k of code). Both modules must
// export it (src/browserCoreParser.test.ts); trimming its code saved 1.1k of the
// 2.3k overshoot, and no further trim fits without dropping a rule.
// 2026-11-27, paste reads as the page reader (GH #1051): the parser ceiling
// moved to 1,570,000 (the measured 1,566,526 rounded up to the next 5,000) for
// the page reader's outline (`pasted_outline_json`), which the parser module
// did not link before (+9.8k). The reader's refusal diagnostic is left out;
// any further trim would split pasted blocks a second way.
export const wasmArtifacts = [
  { dir: "src/render/wasm", feature: null, maxBytes: 1_570_000 },
  { dir: "src/render/wasm-core", feature: "core", maxBytes: 4_794_000 },
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

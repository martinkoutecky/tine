import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const vendorDir = join(root, "src", "devtools", "lsdoc-diff", "vendor");
const metadata = JSON.parse(readFileSync(join(vendorDir, "refs.source.json"), "utf8"));
const cargo = readFileSync(join(root, "crates", "tine-core", "Cargo.toml"), "utf8");
const pin = cargo.match(/lsdoc\s*=\s*\{[^}]*\btag\s*=\s*"([^"]+)"/)?.[1];
if (!pin) throw new Error("could not read tine-core's lsdoc tag");

const bytes = readFileSync(join(vendorDir, "refs.mjs"));
const digest = createHash("sha256").update(bytes).digest("hex");
const problems = [];
if (metadata.lsdocTag !== pin) {
  problems.push(`Cargo pins ${pin}, but the vendored reference oracle came from ${metadata.lsdocTag}`);
}
if (metadata.sha256 !== digest) {
  problems.push(`refs.mjs hash is ${digest}, expected ${metadata.sha256}`);
}
if (problems.length) {
  throw new Error(
    `Stale or modified lsdoc reference oracle:\n  ${problems.join("\n  ")}\n` +
    `Copy ${metadata.source} from the pinned lsdoc release and update refs.source.json.`,
  );
}

console.log(`lsdoc oracle OK: ${pin} refs.mjs ${digest.slice(0, 12)}`);

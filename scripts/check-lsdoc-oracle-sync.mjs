import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const vendorDir = join(root, "src", "devtools", "lsdoc-diff", "vendor");
const attributes = readFileSync(join(root, ".gitattributes"), "utf8");
// Git checks `.gitattributes` out using the platform's native line endings
// unless the file governs itself. Windows CI therefore sees CRLF here even
// though the rules correctly require LF for the vendored oracle bytes.
const attributeLines = attributes.split(/\r?\n/);
const metadata = JSON.parse(readFileSync(join(vendorDir, "oracle.source.json"), "utf8"));
const cargo = readFileSync(join(root, "crates", "tine-core", "Cargo.toml"), "utf8");
const pin = cargo.match(/lsdoc\s*=\s*\{[^}]*\btag\s*=\s*"([^"]+)"/)?.[1];
if (!pin) throw new Error("could not read tine-core's lsdoc tag");

const problems = [];
for (const pattern of [
  "src/devtools/lsdoc-diff/vendor/*.mjs text eol=lf",
  "src/devtools/lsdoc-diff/vendor/mldoc.js text eol=lf",
  "src/devtools/lsdoc-diff/vendor/oracle.source.json text eol=lf",
]) {
  if (!attributeLines.includes(pattern)) {
    problems.push(`.gitattributes is missing: ${pattern}`);
  }
}
if (metadata.lsdocTag !== pin) {
  problems.push(`Cargo pins ${pin}, but the oracle bundle came from ${metadata.lsdocTag}`);
}
if (!/^\d+\.\d+\.\d+$/.test(metadata.mldocVersion)) {
  problems.push(`invalid pinned mldoc version: ${metadata.mldocVersion}`);
}
for (const [file, entry] of Object.entries(metadata.files ?? {})) {
  const digest = createHash("sha256").update(readFileSync(join(vendorDir, file))).digest("hex");
  if (entry.sha256 !== digest) {
    problems.push(`${file} hash is ${digest}, expected ${entry.sha256} from ${entry.source}`);
  }
}
for (const required of ["normalize.mjs", "compare.mjs", "refs.mjs", "mldoc.js"]) {
  if (!metadata.files?.[required]) problems.push(`oracle manifest does not pin ${required}`);
}
if (metadata.files?.["mldoc.js"]?.source !== `npm:mldoc@${metadata.mldocVersion}/index.js`) {
  problems.push("mldoc.js source does not match mldocVersion");
}
if (problems.length) {
  throw new Error(
    `Stale or modified lsdoc oracle bundle:\n  ${problems.join("\n  ")}\n` +
    "Copy every listed source from the pinned releases and update oracle.source.json.",
  );
}

console.log(`lsdoc oracle bundle OK: ${pin}, mldoc ${metadata.mldocVersion}, ${Object.keys(metadata.files).length} pinned files`);

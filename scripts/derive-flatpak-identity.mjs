#!/usr/bin/env node
// Derive the Flatpak build inputs for the identity this tree ships.
//
// The checked-in flatpak/ files carry the RELEASED identity (they are what a
// Flathub submission would use). Every other identity is derived from them at
// build time, so each fact (the build recipe, the permissions, the description)
// has exactly one source of truth and only the identity-bearing facts differ:
// application id, product name, and the release list.
//
//   node scripts/derive-flatpak-identity.mjs --identity <release|experiment>
//
// `--identity` must name the identity the tree ships (src-tauri/app-identity.json
// `ship`); anything else is refused, because the packaged binary reads its
// identity from that switch and a mismatched Flatpak id would give it the other
// app's data directories. The released path is unchanged: it writes only the
// CI manifest, byte-for-byte what flatpak.yml used to derive inline.
//
// Outputs (all untracked; docs/app-identity.md):
//   flatpak/<id>.ci.yml                      manifest that builds THIS checkout
//   flatpak/derived/<id>.desktop             (non-released identities only)
//   flatpak/derived/<id>.metainfo.xml        (non-released identities only)
// The metainfo release date is the commit date (resolveReleaseDate).
// and, with $GITHUB_OUTPUT set, `manifest`, `app-id`, `bundle`.
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { ROOT, readSwitch } from "./lib/app-identity.mjs";

const RELEASED = "release";

/**
 * Pure derivation. Returns { id, productName, manifestPath, files } where `files`
 * maps repo-relative output paths to their text. Throws on a mismatched identity
 * or when a stable source file no longer has the shape the derivation relies on.
 */
export function deriveFlatpak({ root = ROOT, identity, date, version }) {
  const sw = readSwitch(root);
  if (!sw.identities[identity]) {
    throw new Error(`unknown identity "${identity}" (expected one of ${Object.keys(sw.identities).join(", ")})`);
  }
  if (sw.ship !== identity) {
    throw new Error(
      `Flatpak was asked to package the ${identity} identity, but this tree ships the ${sw.ship} identity ` +
        `(src-tauri/app-identity.json). Package the identity the tree ships.`
    );
  }
  const stable = sw.identities[RELEASED];
  const target = sw.identities[identity];
  const stableId = stable.identifier;
  const read = (name) => fs.readFileSync(path.join(root, "flatpak", name), "utf8");
  const idPattern = new RegExp(`${stableId.replace(/\./g, "\\.")}(?!\\w)`, "g");
  const swap = (text, what) => {
    if (!idPattern.test(text)) throw new Error(`${what}: no ${stableId} to derive from`);
    idPattern.lastIndex = 0;
    return text.replace(idPattern, target.identifier);
  };
  const released = identity === RELEASED;
  // Where the derived manifest installs the desktop entry and metainfo from.
  const installDir = released ? "flatpak" : "flatpak/derived";

  let manifest = read(`${stableId}.yml`);
  // Replace the Flathub git source (pinned to a released tag) with a local `dir`
  // source so CI builds THIS commit. The vendored cargo/node source files are
  // unchanged (referenced relative to the manifest dir).
  const at = manifest.indexOf("    sources:");
  if (at < 0) throw new Error("manifest has no module `sources:` block to replace");
  manifest =
    manifest.slice(0, at) +
    "    sources:\n      - type: dir\n        path: ..\n      - cargo-sources.json\n      - node-sources.json\n";
  if (!released) {
    manifest = manifest
      .replaceAll(`flatpak/${stableId}.desktop`, `${installDir}/${stableId}.desktop`)
      .replaceAll(`flatpak/${stableId}.metainfo.xml`, `${installDir}/${stableId}.metainfo.xml`);
    manifest = swap(manifest, "manifest");
  }
  const files = { [`flatpak/${target.identifier}.ci.yml`]: manifest };

  if (!released) {
    let desktop = swap(read(`${stableId}.desktop`), "desktop entry");
    const named = desktop.replace(/^Name=.*$/m, `Name=${target.productName}`);
    if (named === desktop) throw new Error("desktop entry has no Name= line");
    files[`${installDir}/${target.identifier}.desktop`] = named;

    let meta = swap(read(`${stableId}.metainfo.xml`), "metainfo");
    const renamed = meta.replace(/<name>[^<]*<\/name>/, `<name>${target.productName}</name>`);
    if (renamed === meta) throw new Error("metainfo has no <name> element");
    // The stable release history is not this build's history.
    if (!/<releases>[\s\S]*?<\/releases>/.test(meta)) throw new Error("metainfo has no <releases> block");
    meta = renamed.replace(
      /<releases>[\s\S]*?<\/releases>/,
      `<releases>\n    <release version="${version}" date="${date}"/>\n  </releases>`
    );
    files[`${installDir}/${target.identifier}.metainfo.xml`] = meta;
  }
  return {
    id: target.identifier,
    productName: target.productName,
    manifestPath: `flatpak/${target.identifier}.ci.yml`,
    files,
  };
}

/**
 * The metainfo release date: the SOURCE commit's date, never the build date, so
 * rebuilding one SHA derives the same files. `--date`, then SOURCE_DATE_EPOCH
 * (the reproducible-builds convention), then the HEAD commit time. Throws when
 * none is available rather than guessing "today".
 */
export function resolveReleaseDate({ explicit, env = process.env, gitCommitDate = () => commitDate() } = {}) {
  const day = (ms) => new Date(ms).toISOString().slice(0, 10);
  if (explicit) {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(explicit)) throw new Error(`--date must be YYYY-MM-DD, got "${explicit}"`);
    return explicit;
  }
  const epoch = env.SOURCE_DATE_EPOCH;
  if (epoch !== undefined && epoch !== "") {
    if (!/^\d+$/.test(epoch)) throw new Error(`SOURCE_DATE_EPOCH must be whole seconds, got "${epoch}"`);
    return day(Number(epoch) * 1000);
  }
  const committed = gitCommitDate();
  if (!committed) {
    throw new Error("no release date: pass --date, set SOURCE_DATE_EPOCH, or run inside the git checkout");
  }
  return committed;
}

function commitDate() {
  try {
    // %cs is the committer date as YYYY-MM-DD in the committer's timezone.
    const out = execFileSync("git", ["show", "-s", "--format=%cs", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
    return /^\d{4}-\d{2}-\d{2}$/.test(out) ? out : null;
  } catch {
    return null;
  }
}

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === "--identity") out.identity = argv[(i += 1)];
    else if (argv[i] === "--date") out.date = argv[(i += 1)];
    else throw new Error(`unknown argument ${argv[i]}`);
  }
  return out;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const args = parseArgs(process.argv.slice(2));
    if (!args.identity) throw new Error("usage: derive-flatpak-identity.mjs --identity <release|experiment> [--date YYYY-MM-DD]");
    const conf = JSON.parse(fs.readFileSync(path.join(ROOT, "src-tauri/tauri.conf.json"), "utf8"));
    const date = resolveReleaseDate({ explicit: args.date });
    const derived = deriveFlatpak({ identity: args.identity, date, version: conf.version });
    for (const [file, text] of Object.entries(derived.files)) {
      fs.mkdirSync(path.dirname(path.join(ROOT, file)), { recursive: true });
      fs.writeFileSync(path.join(ROOT, file), text);
      console.log(`wrote ${file}`);
    }
    const product = derived.productName.replace(/\s+/g, "-");
    const outputs = { manifest: derived.manifestPath, "app-id": derived.id, product };
    if (process.env.GITHUB_OUTPUT) {
      fs.appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(outputs).map(([k, v]) => `${k}=${v}\n`).join(""));
    }
  } catch (error) {
    console.error(`::error::${error.message}`);
    process.exit(1);
  }
}

import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";

export const VIEWER_ZIP = "quick-share-viewer-g2.zip";
export const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
export function fixtureFingerprint(root, generation = 1) {
  const fixtures = path.join(root, `tests/fixtures/quick-share/g${generation}`);
  const hash = createHash("sha256");
  for (const name of filesUnder(fixtures)) hash.update(name + "\0").update(fs.readFileSync(path.join(fixtures, name)));
  return hash.digest("hex");
}
export function filesUnder(dir, prefix = "") {
  return fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name)).flatMap((entry) => {
    const name = prefix + entry.name;
    if (entry.isSymbolicLink()) throw new Error(`viewer symlink: ${name}`);
    return entry.isDirectory() ? filesUnder(path.join(dir, entry.name), `${name}/`) : [name];
  });
}
export function fileHashes(dir) {
  return Object.fromEntries(filesUnder(dir).filter((name) => name !== "viewer.json").map((name) => [name, sha256(fs.readFileSync(path.join(dir, name)))]));
}
export function validateViewer(dir, generation = 1) {
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, "viewer.json"), "utf8"));
  validateViewerGeneration(manifest, generation);
  if (typeof manifest.built_from !== "string" || !manifest.built_from) throw new Error("viewer built_from missing");
  const actual = fileHashes(dir);
  if (!actual["index.html"] || JSON.stringify(Object.entries(actual).sort()) !== JSON.stringify(Object.entries(manifest.files ?? {}).sort())) throw new Error("viewer file hashes mismatch");
  return manifest;
}

export function validateViewerGeneration(manifest, generation) {
  for (const field of ["generation", "link", "envelope", "document", "snapshot_schema"]) {
    const expected = field === "envelope" ? 1 : generation;
    if (manifest[field] !== expected) throw new Error(`viewer ${field} mismatch`);
  }
}

/** Only the entry document and its manifest are mutable. All other names must
 * keep their bytes forever; an earlier tab can request them at any time. */
export function mergeViewerBuild(source, destination, generation = 1) {
  validateViewer(source, generation);
  fs.mkdirSync(destination, { recursive: true });
  for (const name of filesUnder(source)) {
    const target = path.join(destination, name);
    const bytes = fs.readFileSync(path.join(source, name));
    if (!["index.html", "viewer.json"].includes(name) && fs.existsSync(target) && !fs.readFileSync(target).equals(bytes)) throw new Error(`viewer same-name/different-bytes collision: ${name}`);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, bytes);
  }
}

/** GitHub Pages serves the root 404 document at missing locator paths. */
export function stageViewerBuild(source, website, generation) {
  const destination = path.join(website, "s", String(generation));
  mergeViewerBuild(source, destination, generation);
  if (generation === 2) fs.copyFileSync(path.join(destination, "index.html"), path.join(website, "404.html"));
}

export function unpackViewer(zip, destination, expectedHash, generation = 1) {
  if (!/^[a-f0-9]{64}$/.test(expectedHash) || sha256(fs.readFileSync(zip)) !== expectedHash) throw new Error("viewer zip SHA-256 mismatch");
  // Python's stdlib avoids an npm archive dependency. Validate the complete
  // member list before writing: traversal, duplicates, links and bombs refuse.
  execFileSync("python3", ["-c", `import zipfile,sys,pathlib,stat
z=zipfile.ZipFile(sys.argv[1]); seen=set(); total=0
for i in z.infolist():
 p=pathlib.PurePosixPath(i.filename); total+=i.file_size
 if p.is_absolute() or '..' in p.parts or '\\\\' in i.filename or i.filename in seen or stat.S_ISLNK(i.external_attr>>16) or total>268435456: raise ValueError('unsafe viewer zip')
 seen.add(i.filename)
z.extractall(sys.argv[2])`, zip, destination]);
  return validateViewer(destination, generation);
}

export function packViewer(directory, zip, generation = 1) {
  validateViewer(directory, generation);
  execFileSync("python3", ["-c", `import zipfile,pathlib,sys
root=pathlib.Path(sys.argv[1])
with zipfile.ZipFile(sys.argv[2],'w',compression=zipfile.ZIP_DEFLATED,compresslevel=9) as z:
 for file in sorted(root.rglob('*')):
  if file.is_file(): z.write(file,file.relative_to(root).as_posix())`, directory, zip]);
}

export function validatePin(pin) {
  if (!pin || !/^[a-f0-9]{64}$/.test(pin.sha256 ?? "")) throw new Error("viewer pin needs a zip SHA-256");
  if (pin.tag && /^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(pin.tag) && !pin.run_id) return;
  if (!pin.tag && /^[1-9][0-9]*$/.test(String(pin.run_id ?? ""))) return;
  throw new Error("viewer pin needs exactly one release tag or workflow run_id");
}

export function assertRetainedBuilds(previous, next) {
  for (const [generation, old] of Object.entries(previous.generations ?? {})) {
    for (const pin of old.retained ?? []) {
      // A trial artifact may be archived as a release asset before its Actions
      // retention expires. The retained bytes (zip digest) must remain forever.
      if (!next.generations?.[generation]?.retained?.some((candidate) => candidate.sha256 === pin.sha256)) throw new Error(`cannot remove retained generation ${generation} build ${pin.sha256}`);
    }
  }
}

export async function deployViewers({ manifest, website, download }) {
  if (manifest.schema !== 1 || !manifest.generations || typeof manifest.generations !== "object" || Array.isArray(manifest.generations)) throw new Error("invalid viewer generations manifest");
  for (const [generation, entry] of Object.entries(manifest.generations ?? {})) {
    if (!/^[1-9][0-9]*$/.test(generation)) throw new Error("invalid viewer generation");
    if (!entry.current) {
      if (entry.retained?.length) throw new Error("retained viewer without current");
      continue; // First deployment has not been pinned yet; never invent a digest.
    }
    const retained = entry.retained ?? [];
    if (!retained.some((pin) => pin.sha256 === entry.current.sha256)) throw new Error("current viewer must also be retained");
    for (const pin of [...retained, entry.current]) {
      validatePin(pin);
      const directory = await download(pin, Number(generation));
      stageViewerBuild(directory, website, Number(generation));
    }
  }
}

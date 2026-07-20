import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

function git(root, args, encoding = "utf8") {
  const result = spawnSync("git", args, { cwd: root, encoding, maxBuffer: 32 * 1024 * 1024 });
  if (result.status !== 0) {
    throw result.error || new Error(`git ${args.join(" ")} failed: ${String(result.stderr || "").trim()}`);
  }
  return result.stdout;
}

function sha256(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

function zeroDelimited(buffer) {
  return buffer.toString("utf8").split("\0").filter(Boolean);
}

const generatedTauriSchemaPathspec = ":(exclude)src-tauri/gen/schemas/**";
// Documentation never affects the compiled app binary, so it is not a build
// input for the E2E provenance digest. Excluding it (like the generated Tauri
// schemas above) keeps a docs-only working-tree change — e.g. a Windows checkout
// flipping docs/UI-REGRESSION-TESTING.md — from marking the tree "dirty" and
// failing the advisory Windows smoke, without weakening the binary↔source bind.
const docsPathspec = ":(exclude)docs/**";
const tauriManifestRelativePath = "src-tauri/Cargo.toml";
const tauriManifestPathspec = `:(exclude)${tauriManifestRelativePath}`;

function buildInputPathspecs(excludeTauriManifest = false) {
  return ["--", ".", generatedTauriSchemaPathspec, docsPathspec, ...(excludeTauriManifest ? [tauriManifestPathspec] : [])];
}

function inputState(root, normalizedTauriManifest) {
  const statusPathspecs = buildInputPathspecs(Boolean(normalizedTauriManifest));
  const statusRaw = git(root, ["status", "--porcelain=v1", "-z", "--untracked-files=all", ...statusPathspecs], null);
  const files = zeroDelimited(git(root, ["ls-files", "-co", "--exclude-standard", "-z", ...buildInputPathspecs()], null));
  const entries = files.map((relativePath) => {
    const file = path.resolve(root, relativePath);
    if (file !== root && !file.startsWith(`${root}${path.sep}`)) throw new Error(`Git returned a path outside the worktree: ${relativePath}`);
    try {
      const stat = fs.lstatSync(file);
      const mode = stat.mode & 0o777;
      if (stat.isSymbolicLink()) return { path: relativePath, mode, kind: "symlink", sha256: sha256(fs.readlinkSync(file)) };
      if (stat.isFile()) {
        const bytes = relativePath === tauriManifestRelativePath && normalizedTauriManifest
          ? normalizedTauriManifest
          : fs.readFileSync(file);
        return { path: relativePath, mode, kind: "file", sha256: sha256(bytes) };
      }
      return { path: relativePath, mode, kind: "other" };
    } catch (error) {
      if (error?.code === "ENOENT") return { path: relativePath, kind: "missing" };
      throw error;
    }
  });
  const changes = git(root, ["status", "--porcelain=v1", "--untracked-files=all", ...statusPathspecs])
    .split(/\r?\n/)
    .filter(Boolean);
  return {
    digest: sha256(JSON.stringify({ status: statusRaw.toString("base64"), entries })),
    dirty: statusRaw.length > 0,
    changes,
  };
}

// This exact Git list and digest bind an E2E candidate to its build inputs.
export function buildInputState(root) {
  return inputState(root);
}

// Tauri rewrites Cargo.toml while constructing its native AppInterface. The
// inspect command constructs that same interface without compiling the app, so
// a fresh smoke checkout can derive the exact target-native bytes as the build.
// Always put the caller's bytes back: derivation must be idempotent and must not
// turn receipt validation into a source mutation.
export function deriveTauriManifest(root) {
  const manifest = path.join(root, tauriManifestRelativePath);
  const initialState = buildInputState(root);
  const original = fs.readFileSync(manifest);
  const cli = path.join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
  let result;
  let normalized;
  try {
    result = spawnSync(process.execPath, [cli, "inspect", "wix-upgrade-code"], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 32 * 1024 * 1024,
    });
    normalized = fs.readFileSync(manifest);
  } finally {
    fs.writeFileSync(manifest, original);
  }
  if (result?.status !== 0) {
    throw result?.error || new Error(`Tauri manifest normalization failed: ${String(result?.stderr || "").trim()}`);
  }
  if (!normalized) throw new Error("Tauri manifest normalization did not produce src-tauri/Cargo.toml");
  if (buildInputState(root).digest !== initialState.digest) {
    throw new Error("Tauri manifest normalization changed build inputs while deriving src-tauri/Cargo.toml");
  }
  return normalized;
}

// Cargo.toml's canonical bytes replace only that file's content hash and Git
// status in the digest. Its raw pre-build dirty state is still reported, while
// every other file remains byte-, mode-, and status-exact.
export function normalizedBuildInputState(root, expectedTauriManifest) {
  const normalized = deriveTauriManifest(root);
  if (expectedTauriManifest && !normalized.equals(expectedTauriManifest)) {
    throw new Error("src-tauri/Cargo.toml derives a different target-native Tauri manifest");
  }
  const canonical = inputState(root, normalized);
  const raw = buildInputState(root);
  return {
    ...canonical,
    dirty: raw.dirty,
    changes: raw.changes,
    tauriManifest: normalized,
  };
}

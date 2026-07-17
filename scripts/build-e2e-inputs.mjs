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

// This exact Git list and digest bind an E2E candidate to its build inputs.
export function buildInputState(root) {
  const statusRaw = git(root, ["status", "--porcelain=v1", "-z", "--untracked-files=all"], null);
  const files = zeroDelimited(git(root, ["ls-files", "-co", "--exclude-standard", "-z"], null));
  const entries = files.map((relativePath) => {
    const file = path.resolve(root, relativePath);
    if (file !== root && !file.startsWith(`${root}${path.sep}`)) throw new Error(`Git returned a path outside the worktree: ${relativePath}`);
    try {
      const stat = fs.lstatSync(file);
      const mode = stat.mode & 0o777;
      if (stat.isSymbolicLink()) return { path: relativePath, mode, kind: "symlink", sha256: sha256(fs.readlinkSync(file)) };
      if (stat.isFile()) return { path: relativePath, mode, kind: "file", sha256: sha256(fs.readFileSync(file)) };
      return { path: relativePath, mode, kind: "other" };
    } catch (error) {
      if (error?.code === "ENOENT") return { path: relativePath, kind: "missing" };
      throw error;
    }
  });
  const changes = git(root, ["status", "--porcelain=v1", "--untracked-files=all"])
    .split(/\r?\n/)
    .filter(Boolean);
  return {
    digest: sha256(JSON.stringify({ status: statusRaw.toString("base64"), entries })),
    dirty: statusRaw.length > 0,
    changes,
  };
}

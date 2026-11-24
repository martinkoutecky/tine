// The one answer to "which bundle does a browser-preview journey run against":
// the browser-mock bundle (`npm run build:mock`, ADR 0081), never the production
// `dist/`. Since browser core 5b the production build carries no browser backend
// ("This build has no browser backend"), so a journey that served `dist/` saw an
// empty app whenever the last build was a production one (nightly 2026-11-24).
//
// The mock bundle lives in its own directory under the ignored `artifacts/`, keyed
// by the build-input digest the native E2E receipt already uses, so it never
// replaces the production `dist/` a native binary embeds, and a second journey on
// the same tree reuses it instead of rebuilding. Callers just start the preview.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { buildInputState } from "../build-e2e-inputs.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const STAMP = "tine-mock-bundle.json";
const STALE_MS = 60 * 60 * 1000;

function readStamp(dir) {
  try { return JSON.parse(fs.readFileSync(path.join(dir, STAMP), "utf8")); } catch { return undefined; }
}

/** The mock bundle for the current build inputs: reused when this tree already
 * built it, otherwise built once (one `vite build --mode mock`, into a private
 * directory renamed into place). Throws with the build's output on failure. */
export function ensureMockBundle({ root = ROOT } = {}) {
  const digest = buildInputState(root).digest;
  const base = path.join(root, "artifacts/browser-mock");
  const dir = path.join(base, digest.slice(0, 16));
  if (readStamp(dir)?.buildInputDigest === digest && fs.existsSync(path.join(dir, "index.html"))) return dir;
  fs.mkdirSync(base, { recursive: true });
  const staging = fs.mkdtempSync(path.join(base, "building-"));
  const npm = process.platform === "win32" ? "npm.cmd" : "npm";
  const result = spawnSync(npm, ["run", "build:mock", "--", "--outDir", staging, "--emptyOutDir"],
    { cwd: root, encoding: "utf8", shell: process.platform === "win32", maxBuffer: 64 * 1024 * 1024 });
  if (result.status !== 0) {
    throw new Error(`browser-preview journeys need the mock bundle, and \`npm run build:mock\` failed (exit ${result.status ?? result.signal}):\n`
      + `${String(result.stdout || "").slice(-3000)}\n${String(result.stderr || "").slice(-3000)}`);
  }
  fs.writeFileSync(path.join(staging, STAMP), `${JSON.stringify({ mode: "mock", buildInputDigest: digest, builtAt: new Date().toISOString() })}\n`);
  try {
    fs.renameSync(staging, dir);
  } catch (error) {
    // Another journey on the same inputs installed it first: use theirs.
    if (readStamp(dir)?.buildInputDigest !== digest) throw error;
    fs.rmSync(staging, { recursive: true, force: true });
  }
  // Older bundles (other inputs) and abandoned staging directories are scratch.
  for (const entry of fs.readdirSync(base)) {
    const other = path.join(base, entry);
    if (other === dir) continue;
    try { if (Date.now() - fs.statSync(other).mtimeMs > STALE_MS) fs.rmSync(other, { recursive: true, force: true }); } catch {}
  }
  return dir;
}

/** Serve the mock bundle with `vite preview` and wait until it answers.
 * Returns `{ url, server, close }`; `close()` ends the preview server. */
export async function startMockPreview({ port, host = "127.0.0.1", env = process.env, root = ROOT, tries = 120 } = {}) {
  if (!Number.isInteger(port) || port <= 0) throw new Error(`startMockPreview needs a port, got ${port}`);
  const outDir = ensureMockBundle({ root });
  const server = spawn(process.execPath, [path.join(root, "node_modules/vite/bin/vite.js"), "preview", "--outDir", outDir,
    "--host", host, "--port", String(port), "--strictPort"], { cwd: root, env, stdio: "ignore", detached: process.platform !== "win32" });
  const close = () => {
    try { if (process.platform === "win32") server.kill(); else process.kill(-server.pid, "SIGTERM"); } catch {}
  };
  let exited = false;
  server.once("exit", () => { exited = true; });
  const url = `http://${host}:${port}/`;
  for (let attempt = 0; attempt < tries; attempt++) {
    try { if ((await fetch(url)).ok) return { url, server, close }; } catch {}
    if (exited) break;
    await sleep(250);
  }
  close();
  throw new Error(`vite preview of the mock bundle ${path.relative(root, outDir)} did not answer at ${url}`);
}

#!/usr/bin/env node
// The launch-backup syscall gate (docs/storage-contract.md, Graph backups;
// REVIEW-backup-zip F8/D7). Runs the production-path test
// `backup::tests::syscall_gate_scenarios` under strace and checks, per
// scenario, the order of the real fsync, rename and unlink calls:
//   - a zip partial is fsynced before its rename;
//   - after the rename, `<id>.zips/`, `backups/` and app data are fsynced,
//     in that order;
//   - before a snapshot is unlinked, a keeper is settled: a publication's
//     chain, or a zip's fsync followed by the same chain;
//   - the unchanged launch publishes nothing (no rename, no unlink) and
//     settles its baseline (its fsync, then the chain).
// The in-process op record (`record_backup_op`) cannot see a removed
// `sync_all`; this gate can. Run it for every backup-touching change:
//   source scripts/env.sh && node scripts/check-backup-syscalls.mjs
// strace comes from PATH or TINE_STRACE. Missing strace fails under CI=true
// and is reported as skipped locally.
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const TEST = "backup::tests::syscall_gate_scenarios";
const MARKER = "/tine-syscall-gate-marker/";
const SCENARIOS = {
  "first-publication": { renames: 1, unlinks: 0 },
  "unchanged-settled": { renames: 0, unlinks: 0, settles: true },
  "changed-publication": { renames: 1, unlinks: 0 },
  "publication-with-prune": { renames: 1, unlinks: 1 },
  "publication-keep-12": { renames: 1, unlinks: 0 },
  "settings-prune": { renames: 0, unlinks: 2 },
};

/** One traced call: `{ kind: "fsync" | "rename" | "unlink", path, to?, ok }`. */
export function parseTrace(text) {
  const events = [];
  for (const raw of text.split("\n")) {
    const line = raw.replace(/^(\[pid\s+)?\d+\]?\s+/, "");
    const ok = /\)\s+=\s+0\s*$/.test(line);
    let match;
    if ((match = line.match(/^f(?:data)?sync\(\d+<([^>]*)>\)/))) {
      events.push({ kind: "fsync", path: match[1], ok });
    } else if ((match = line.match(/^rename(?:at2?)?\(.*?"([^"]+)",.*?"([^"]+)"/))) {
      events.push({ kind: "rename", path: match[1], to: match[2], ok });
    } else if ((match = line.match(/^unlinkat\(\d+<([^>]*)>, "([^"]+)"/))) {
      events.push({ kind: "unlink", path: path.posix.join(match[1], match[2]), ok });
    } else if ((match = line.match(/^unlink(?:at)?\((?:AT_FDCWD<[^>]*>, )?"([^"]+)"/))) {
      events.push({ kind: "unlink", path: match[1], ok });
    }
  }
  return events;
}

const isZipsDir = (p) => p.endsWith(".zips");
const isPartial = (p) => path.posix.basename(p).startsWith(".partial-");

/** The violations in a parsed trace; empty when the gate holds. */
export function check(events) {
  const problems = [];
  const segments = new Map();
  let current = null;
  for (const event of events) {
    if (event.kind === "unlink" && event.path.startsWith(MARKER)) {
      current = event.path.slice(MARKER.length);
      segments.set(current, []);
    } else if (current && event.ok) {
      segments.get(current).push(event);
    }
  }
  if (!segments.has("end")) problems.push("the trace never reached the end marker");
  for (const [name, expected] of Object.entries(SCENARIOS)) {
    const segment = segments.get(name);
    if (!segment) {
      problems.push(`${name}: no marker in the trace`);
      continue;
    }
    problems.push(...checkSegment(segment, expected).map((problem) => `${name}: ${problem}`));
  }
  return problems;
}

/** Chain = fsync of `<id>.zips/`, `backups/`, app data, in order, from index `from`. */
function chainEnd(segment, from, zips) {
  const chain = [zips, path.posix.dirname(zips), path.posix.dirname(path.posix.dirname(zips))];
  let at = from;
  for (const dir of chain) {
    while (at < segment.length && !(segment[at].kind === "fsync" && segment[at].path === dir)) {
      if (segment[at].kind !== "fsync") return -1;
      at += 1;
    }
    if (at === segment.length) return -1;
    at += 1;
  }
  return at;
}

function checkSegment(segment, expected) {
  const problems = [];
  const renames = segment.filter((e) => e.kind === "rename");
  const unlinks = segment.filter(
    (e) => e.kind === "unlink" && isZipsDir(path.posix.dirname(e.path)) && !isPartial(e.path) && e.path.endsWith(".zip"),
  );
  if (renames.length !== expected.renames) {
    problems.push(`expected ${expected.renames} publication rename(s), saw ${renames.length}`);
  }
  if (unlinks.length !== expected.unlinks) {
    problems.push(`expected ${expected.unlinks} snapshot unlink(s), saw ${unlinks.length}`);
  }
  // Settled keepers so far: the zip path and the index after its chain.
  const settled = [];
  segment.forEach((event, at) => {
    if (event.kind === "rename") {
      const zips = path.posix.dirname(event.to);
      if (!isPartial(event.path) || !isZipsDir(zips)) {
        problems.push(`unexpected rename ${event.path} -> ${event.to}`);
        return;
      }
      const synced = segment.slice(0, at).some((e) => e.kind === "fsync" && e.path === event.path);
      if (!synced) problems.push(`partial ${path.posix.basename(event.path)} renamed without an fsync`);
      const end = chainEnd(segment, at + 1, zips);
      if (end < 0) problems.push(`no zips -> backups -> app-data fsync chain after the rename of ${path.posix.basename(event.to)}`);
      else settled.push({ path: event.to, from: end });
    } else if (event.kind === "fsync" && event.path.endsWith(".zip") && !isPartial(event.path)) {
      const end = chainEnd(segment, at + 1, path.posix.dirname(event.path));
      if (end > 0) settled.push({ path: event.path, from: end });
      else if (expected.settles) problems.push(`baseline ${path.posix.basename(event.path)} fsynced without its chain`);
    }
  });
  if (expected.settles && settled.length === 0) problems.push("the unchanged launch did not settle its baseline");
  segment.forEach((event, at) => {
    if (!unlinks.includes(event)) return;
    const keeper = settled.find((k) => k.from <= at && k.path !== event.path);
    if (!keeper) problems.push(`${path.posix.basename(event.path)} unlinked before a keeper was settled`);
  });
  return problems;
}

function findStrace() {
  const candidates = [process.env.TINE_STRACE, "strace"].filter(Boolean);
  for (const candidate of candidates) {
    if (spawnSync(candidate, ["-V"], { stdio: "ignore" }).status === 0) return candidate;
  }
  return null;
}

function testBinary(root) {
  const built = spawnSync("cargo", ["test", "-p", "tine", "--lib", "--no-run", "--message-format=json"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 512 << 20,
    stdio: ["ignore", "pipe", "inherit"],
  });
  if (built.status !== 0) throw new Error("cargo test --no-run failed");
  const executables = built.stdout
    .split("\n")
    .filter((line) => line.startsWith("{"))
    .map((line) => JSON.parse(line))
    .filter((m) => m.reason === "compiler-artifact" && m.executable && m.profile?.test && m.target?.name === "tine_lib")
    .map((m) => m.executable);
  if (executables.length !== 1) throw new Error(`expected one tine_lib test binary, found ${executables.length}`);
  return executables[0];
}

function main() {
  const strace = findStrace();
  if (!strace) {
    const message = "check-backup-syscalls: strace is not installed (set TINE_STRACE or install strace)";
    if (process.env.CI === "true") {
      console.error(`${message}; the gate cannot be skipped in CI`);
      process.exit(1);
    }
    console.warn(`${message}; SKIPPED locally`);
    return;
  }
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const binary = testBinary(root);
  const dir = mkdtempSync(path.join(tmpdir(), "tine-syscall-gate-"));
  const log = path.join(dir, "trace.log");
  try {
    const run = spawnSync(
      strace,
      [
        "-f",
        "-yy",
        "-s",
        "512",
        "-o",
        log,
        "-e",
        "trace=fsync,fdatasync,rename,renameat,renameat2,unlink,unlinkat",
        binary,
        "--exact",
        TEST,
        "--test-threads=1",
      ],
      { cwd: root, stdio: "inherit" },
    );
    if (run.status !== 0) throw new Error(`${TEST} failed under strace (status ${run.status})`);
    const problems = check(parseTrace(readFileSync(log, "utf8")));
    if (problems.length) {
      console.error("check-backup-syscalls: FAILED");
      for (const problem of problems) console.error(`  - ${problem}`);
      process.exitCode = 1;
      return;
    }
    console.log(`check-backup-syscalls: ok (${Object.keys(SCENARIOS).length} scenarios)`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(`check-backup-syscalls: ${error.message}`);
    process.exit(1);
  }
}

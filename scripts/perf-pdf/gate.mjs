// PDF viewer perf gate (docs/pdf-perf-harness.md): base binary vs candidate
// binary on the same machine, before a stable cut.
//
//   node scripts/perf-pdf/gate.mjs --base <tine binary> --candidate <tine binary> [--out <dir>]
//     [--sessions 3] [--dprs 1,2] [--load-max 4] [--load-wait-min 15] [--cpus <taskset list>]
//   node scripts/perf-pdf/gate.mjs --report-only <out>   # re-render report.md/json from <out>/raw
//
// Runs the trimmed scenario set (run.mjs --gate, Tine's viewer only) in fresh
// sessions, base and candidate interleaved per DPR and repetition (order
// alternating) so machine drift hits both. Writes <out>/report.md and
// <out>/report.json; raw rows and logs go to <out>/raw/.
//
// Exit: 0 no regression · 1 regression · 2 incomplete (a session failed, or the
// candidate has no probe) · 3 machine stayed loaded · 4 no verdict: the base
// has no probe (a release from before D-20261021-PV2), candidate-only report.
import { spawn, spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { ensureAssets } from "./assets.mjs";
import { FAMILIES, resultTable, sideOf, verdict } from "./verdict.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "../..");
const arg = (name, dflt) => { const i = process.argv.indexOf(`--${name}`); return i >= 0 ? process.argv[i + 1] : dflt; };
const BASE = arg("base"), CAND = arg("candidate"), REPORT_ONLY = arg("report-only");
if (!REPORT_ONLY && (!BASE || !CAND)) { console.error("usage: gate.mjs --base <binary> --candidate <binary> [--out dir] | --report-only <out>"); process.exit(2); }
const OUT = path.resolve(REPORT_ONLY || arg("out", path.join(ROOT, "target/perf-pdf-gate", new Date().toISOString().replace(/[:.]/g, "-"))));
const RAW = path.join(OUT, "raw");
// --report-only re-renders an earlier run (e.g. after a threshold change) from its raw rows.
const prior = REPORT_ONLY ? JSON.parse(fs.readFileSync(path.join(OUT, "report.json"), "utf8")) : null;
const settings = prior?.settings ?? { sessions: Number(arg("sessions", "3")), dprs: arg("dprs", "1,2").split(",").map(Number),
  loadMax: Number(arg("load-max", "4")), loadWaitMin: Number(arg("load-wait-min", "15")), cpus: arg("cpus", "") };
const { sessions: SESSIONS, dprs: DPRS, loadMax: LOAD_MAX, loadWaitMin: LOAD_WAIT_MIN, cpus: CPUS } = settings;
const SESSION_TIMEOUT_MS = 15 * 60_000;
const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);
const load1 = () => os.loadavg()[0];
const loads = () => os.loadavg().map((x) => Math.round(x * 100) / 100);

let current = null; // the running session's process group
let binaries = prior?.binaries, sessions = prior?.sessions ?? [], baseNoProbe = prior?.baseNoProbe ?? false, abort = prior?.abort ?? null;
const started = Date.now();
if (!prior) {
// ---------------------------------------------------------------- preflight
const problems = [];
for (const tool of ["xvfb-run", "tauri-driver", "xdotool"]) {
  if (spawnSync("sh", ["-c", `command -v ${tool}`]).status !== 0) problems.push(`missing host tool: ${tool}`);
}
const webkitDriver = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
if (!fs.existsSync(webkitDriver)) problems.push(`missing WebKitWebDriver at ${webkitDriver}`);
for (const [label, bin] of [["base", BASE], ["candidate", CAND]]) {
  try { fs.accessSync(bin, fs.constants.X_OK); } catch { problems.push(`${label} binary is not executable: ${bin}`); }
}
// One harness session per machine: a second one shares the CPUs and driver ports.
// Match argv, not the command text: `pgrep -f` also matches any shell whose
// command line merely mentions the script.
const other = fs.readdirSync("/proc").filter((pid) => /^\d+$/.test(pid)).filter((pid) => {
  try {
    const argv = fs.readFileSync(`/proc/${pid}/cmdline`, "utf8").split("\0");
    return path.basename(argv[0]) === "node" && argv.slice(1).some((a) => a.endsWith("perf-pdf/run.mjs"));
  } catch { return false; }
});
if (other.length) problems.push(`another harness session is running (pids ${other.join(", ")})`);
// pgrep cannot see a second gate that is still waiting for a quiet machine; the lock can.
const LOCK = path.join(os.tmpdir(), "tine-pdf-perf-gate.lock");
const lockHolder = () => { try { const pid = Number(fs.readFileSync(path.join(LOCK, "pid"), "utf8")); process.kill(pid, 0); return pid; } catch { return null; } };
if (!problems.length) {
  try { fs.mkdirSync(LOCK); } catch {
    const pid = lockHolder();
    if (pid) problems.push(`another PDF perf gate is running (pid ${pid}, lock ${LOCK})`);
    else { fs.rmSync(LOCK, { recursive: true, force: true }); fs.mkdirSync(LOCK); }
  }
  if (!problems.length) {
    fs.writeFileSync(path.join(LOCK, "pid"), String(process.pid));
    process.on("exit", () => {
      if (current) try { process.kill(-current.pid, "SIGTERM"); } catch {}
      if (lockHolder() === process.pid) fs.rmSync(LOCK, { recursive: true, force: true });
    });
    // Stopping the gate stops its session too and releases the lock.
    for (const sig of ["SIGINT", "SIGTERM"]) process.on(sig, () => process.exit(2));
  }
}
if (problems.length) { for (const p of problems) console.error(p); process.exit(2); }

const { corpus, dist } = await ensureAssets();
fs.mkdirSync(RAW, { recursive: true });
const sha = (f) => crypto.createHash("sha256").update(fs.readFileSync(f)).digest("hex");
binaries = { base: { path: path.resolve(BASE), sha256: sha(BASE) }, cand: { path: path.resolve(CAND), sha256: sha(CAND) } };

// ---------------------------------------------------------------- sessions
async function waitForQuietMachine() {
  const deadline = Date.now() + LOAD_WAIT_MIN * 60_000;
  let said = false;
  while (load1() >= LOAD_MAX) {
    if (Date.now() > deadline) return false;
    if (!said) { log(`waiting for 1-min load < ${LOAD_MAX} (now ${loads()[0]})`); said = true; }
    await sleep(10_000);
  }
  return true;
}

function runSession(side, dpr, rep) {
  const tag = `${side}-dpr${dpr}-r${rep}`;
  const screen = dpr === 2 ? "3840x2160x24" : "1920x1080x24";
  const cmd = [
    ...(CPUS ? ["taskset", "-c", CPUS] : []),
    "xvfb-run", "-a", "-s", `-screen 0 ${screen}`,
    process.execPath, path.join(HERE, "run.mjs"), "--gate", "--viewers", "ours", "--scenarios", "S1a,S1c,S2",
    "--dpr", String(dpr), "--out", RAW, "--tag", tag,
  ];
  const env = { ...process.env, TINE_APP: binaries[side].path, PDF_CORPUS: corpus, PDFJS_DIST: dist, LANG: "C.UTF-8", LC_ALL: "C.UTF-8" };
  const logFd = fs.openSync(path.join(RAW, `${tag}.log`), "w");
  const t0 = Date.now(), loadStart = loads();
  return new Promise((resolve) => {
    const child = spawn(cmd[0], cmd.slice(1), { cwd: ROOT, env, stdio: ["ignore", logFd, logFd], detached: true });
    current = child;
    const timer = setTimeout(() => {
      try { process.kill(-child.pid, "SIGTERM"); } catch {}
      setTimeout(() => { try { process.kill(-child.pid, "SIGKILL"); } catch {} }, 10_000).unref();
    }, SESSION_TIMEOUT_MS);
    child.on("exit", (code, signal) => {
      clearTimeout(timer);
      current = null;
      fs.closeSync(logFd);
      resolve({ tag, side, dpr, rep, exit: code ?? `signal ${signal}`, wallS: Math.round((Date.now() - t0) / 1000), loadStart, loadEnd: loads() });
    });
  });
}

outer: for (let rep = 1; rep <= SESSIONS; rep++) {
  for (const dpr of DPRS) {
    const order = rep % 2 ? ["base", "cand"] : ["cand", "base"];
    for (const side of order) {
      if (side === "base" && baseNoProbe) continue;
      if (!(await waitForQuietMachine())) { abort = `1-min load stayed >= ${LOAD_MAX} for ${LOAD_WAIT_MIN} min`; break outer; }
      log(`session ${side}-dpr${dpr}-r${rep} (load ${loads().join(" ")})`);
      const s = await runSession(side, dpr, rep);
      sessions.push(s);
      log(`  exit ${s.exit} after ${s.wallS} s`);
      if (s.exit === 4 && side === "base") { baseNoProbe = true; log("base binary emits no probe events: candidate-only report, no verdict"); }
      if (s.exit === 4 && side === "cand") { abort = "the candidate binary emits no __tinePdfProbe events; it cannot be measured"; break outer; }
    }
  }
}

}

// ---------------------------------------------------------------- report
const rows = [];
for (const f of fs.readdirSync(RAW)) {
  if (!f.endsWith(".jsonl") || !sideOf(f)) continue;
  for (const line of fs.readFileSync(path.join(RAW, f), "utf8").split("\n")) if (line.trim()) rows.push(JSON.parse(line));
}
const failed = sessions.filter((s) => s.exit !== 0 && !(s.exit === 4 && s.side === "base"));
// A base without the probe fails its first open by design; that is not an error.
const errored = rows.filter((r) => r.error && !(sideOf(r.tag) === "base" && baseNoProbe && String(r.error).includes("NO_PROBE")));
const v = verdict(rows);
const incomplete = abort != null || failed.length > 0 || errored.length > 0;
const outcome = abort?.startsWith("1-min load") ? { code: 3, text: `ABORTED: ${abort}` }
  : incomplete ? { code: 2, text: `INCOMPLETE: ${abort ?? `${failed.length} failed session(s), ${errored.length} errored trial(s)`}` }
  : baseNoProbe ? { code: 4, text: "NO VERDICT: the base binary has no PDF probe (built before D-20261021-PV2), so only the candidate was measured" }
  : v.regressions.length ? { code: 1, text: `REGRESSION: ${v.regressions.length} metric(s) worse than base beyond the run-to-run spread` }
  : { code: 0, text: `PASS: no regression (${v.improvements.length} improvement(s), ${v.watch.length} watch)` };
const wallMin = prior?.wallMin ?? Math.round((Date.now() - started) / 6000) / 10;
const measureMin = Math.round(sessions.reduce((a, s) => a + s.wallS, 0) / 6) / 10;

const md = [];
md.push(`# PDF perf gate: ${outcome.text}`, "");
md.push(`- base: \`${binaries.base.path}\` (sha256 \`${binaries.base.sha256.slice(0, 16)}…\`)${baseNoProbe ? " **no probe**" : ""}`);
md.push(`- candidate: \`${binaries.cand.path}\` (sha256 \`${binaries.cand.sha256.slice(0, 16)}…\`)`);
md.push(`- host: ${os.hostname()}, ${os.cpus().length} CPUs, pinned: ${CPUS || "no"}; wall time ${wallMin} min (${measureMin} min in sessions, the rest waiting for a quiet machine); sessions per side and DPR: ${SESSIONS}; DPR ${DPRS.join(", ")}`);
md.push(`- rule: regression = candidate's best session worse than base's worst by more than max(base range, floor, rel × base median); see docs/pdf-perf-harness.md`);
md.push("");
if (errored.length) {
  md.push("## Errored trials", "");
  for (const r of errored) md.push(`- ${r.tag} ${r.scenario} ${r.name}: ${String(r.error).split("\n")[0]}`);
  md.push("");
}
if (baseNoProbe) {
  md.push("## Candidate (no verdict)", "", resultTable(v.results, { withBase: false }), "");
} else {
  for (const [title, list] of [["Regressions", v.regressions], ["Improvements", v.improvements], ["Watch (median shift, ranges overlap; not blocking)", v.watch]]) {
    md.push(`## ${title} (${list.length})`, "");
    if (list.length) md.push(resultTable(list), "");
  }
  md.push("## All metrics", "", "Values: median [min–max] over sessions, one value per session.", "", resultTable(v.results), "");
}
md.push("## Sessions", "", "| session | exit | wall s | load at start (1/5/15 min) | load at end |", "|---|---|---|---|---|");
for (const s of sessions) md.push(`| ${s.tag} | ${s.exit} | ${s.wallS} | ${s.loadStart.join(" / ")} | ${s.loadEnd.join(" / ")} |`);
md.push("", "Thresholds (floor, rel): " + Object.entries(FAMILIES).map(([, f]) => `${f.label} ${f.floor}${f.unit} / ${f.rel * 100}%`).join("; "), "");
fs.writeFileSync(path.join(OUT, "report.md"), md.join("\n"));
fs.writeFileSync(path.join(OUT, "report.json"), JSON.stringify({ outcome, binaries, settings, baseNoProbe, abort, wallMin, measureMin, sessions,
  families: FAMILIES, regressions: v.regressions, improvements: v.improvements, watch: v.watch, results: v.results }, null, 1));
log(outcome.text);
log(`report: ${path.join(OUT, "report.md")}`);
process.exit(outcome.code);

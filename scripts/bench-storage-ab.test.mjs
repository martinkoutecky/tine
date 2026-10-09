// Unit tests for the storage A/B harness's pure helpers and the syscall tracer
// (scripts/bench-storage-ab.mjs, docs/bench-storage-ab.md). No app is launched.
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  quantile, median, describe, medianSpreadPct, deltaPct, verdict, NOISY_PCT,
  typedPrefixIn, keyToPublish, allSeenAt, slopePerMinute, summarizeIo, parseNdjson,
} from "./lib/bench-ab-stats.mjs";
import { Adapter } from "./lib/bench-ab-adapters.mjs";
import { SCENARIOS, DEFAULT_SCENARIOS, METRICS, metricInfo } from "./lib/bench-ab-scenarios.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));

test("quantile uses the nearest-rank convention of bench-og-parity", () => {
  assert.equal(quantile([], 0.5), null);
  assert.equal(quantile([5], 0.95), 5);
  assert.equal(quantile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0.95), 10);
  assert.equal(quantile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0.5), 5);
  assert.equal(median([3, 1, 2]), 2);
});

test("describe reports n, median, p95 and range, and ignores non-finite samples", () => {
  assert.equal(describe([]), null);
  assert.equal(describe([NaN, undefined]), null);
  const d = describe([10, 20, 30, NaN]);
  assert.equal(d.n, 3);
  assert.equal(d.median, 20);
  assert.equal(d.min, 10);
  assert.equal(d.max, 30);
  assert.equal(d.rangePct, 100);
});

test("the A/A spread is the relative median difference, symmetric in its arguments", () => {
  const a = describe([100, 100, 100]);
  const b = describe([110, 110, 110]);
  const s = medianSpreadPct(a, b);
  assert.ok(Math.abs(s - (10 / 105) * 100) < 1e-9);
  assert.equal(medianSpreadPct(a, b), medianSpreadPct(b, a));
  assert.equal(medianSpreadPct(a, null), null);
  assert.equal(medianSpreadPct(describe([0]), describe([0])), 0);
  assert.ok(Math.abs(deltaPct(a, b) - 10) < 1e-9);
});

test("verdicts: a change inside the measured floor is noise; a noisy floor suppresses the verdict", () => {
  assert.equal(verdict({ delta: 3, floor: 5 }), "within noise");
  assert.match(verdict({ delta: 30, floor: 5 }), /WORSE/);
  assert.match(verdict({ delta: -30, floor: 5 }), /better/);
  assert.match(verdict({ delta: 30, floor: NOISY_PCT + 1 }), /noisy metric/);
  assert.equal(verdict({ delta: 30, floor: null }), "no A/A floor");
  assert.equal(verdict({ delta: null, floor: 1 }), "n/a");
  assert.match(verdict({ delta: 30, floor: 5, lowerIsBetter: false }), /better/, "higher is better when told so");
  assert.equal(verdict({ delta: 30, floor: 5, absDelta: 1, absFloor: 2 }), "within noise (absolute)");
});

test("typedPrefixIn finds how much of the typed token a published text carries", () => {
  const token = "qzxabc123456";
  assert.equal(typedPrefixIn("- qzxabc123456", token), token.length);
  assert.equal(typedPrefixIn("- qzxabc12", token), 8);
  assert.equal(typedPrefixIn("- unrelated", token), 0);
  assert.equal(typedPrefixIn("", token), 0);
  // an earlier, shorter occurrence must not hide a longer one
  assert.equal(typedPrefixIn("qzx and later qzxabc1234", token), 10);
  // the prefix is not a substring match of the tail: "c123456" alone carries nothing
  assert.equal(typedPrefixIn("- c123456", token), 0);
});

test("keyToPublish: a key is published by the first publish at or after it that carries it", () => {
  const keys = [{ i: 1, t: 100 }, { i: 2, t: 200 }, { i: 3, t: 300 }];
  const publishes = [{ t: 250, k: 2 }, { t: 900, k: 3 }];
  const { latencies, lost } = keyToPublish(keys, publishes);
  assert.deepEqual(latencies, [150, 50, 600]);
  assert.deepEqual(lost, []);
  // a publish that carried less than the key does not count; an unpublished key is lost, not zero
  const r = keyToPublish(keys, [{ t: 250, k: 2 }]);
  assert.deepEqual(r.lost, [3]);
  // a publish before the key cannot publish it
  assert.deepEqual(keyToPublish([{ i: 1, t: 500 }], [{ t: 400, k: 9 }]).lost, [1]);
});

test("allSeenAt is the time the last needle first appears; null when one never does", () => {
  const publishes = [{ t: 10, text: "a x1" }, { t: 30, text: "b x2" }, { t: 20, text: "x3" }];
  assert.equal(allSeenAt(publishes, ["x1", "x3"]), 20);
  assert.equal(allSeenAt(publishes, ["x1", "x2", "x3"]), 30);
  assert.equal(allSeenAt(publishes, ["x1", "missing"]), null);
});

test("slopePerMinute is a least-squares slope in units per minute", () => {
  const pts = [0, 1, 2, 3].map((m) => ({ x: m * 60_000, y: 100 + 5 * m }));
  assert.ok(Math.abs(slopePerMinute(pts) - 5) < 1e-9);
  assert.equal(slopePerMinute([{ x: 0, y: 1 }]), null);
  assert.equal(slopePerMinute([{ x: 5, y: 1 }, { x: 5, y: 2 }]), null);
});

test("summarizeIo attributes only successful work inside the window and the roots", () => {
  const roots = { graph: "/g", appData: "/x" };
  const ev = (t, sys, p, extra = {}) => ({ t, sys, path: p, path2: "", n: 0, ret: 0, isdir: 0, flags: 0, ...extra });
  const events = [
    ev(1, "openat", "/g/pages/a.md.tmp", { flags: 0o100 }),
    ev(2, "write", "/g/pages/a.md.tmp", { n: 100 }),
    ev(3, "fsync", "/g/pages/a.md.tmp"),
    ev(4, "rename", "/g/pages/a.md.tmp", { path2: "/g/pages/a.md" }),
    ev(5, "fsync", "/g/pages", { isdir: 1 }),
    ev(6, "write", "/x/page.tine/draft.json", { n: 40 }),
    ev(7, "write", "/elsewhere/cache", { n: 9999 }),
    ev(8, "write", "/g/pages/a.md.tmp", { n: 5, ret: -28 }),
    ev(99, "write", "/g/pages/late", { n: 1 }),
  ];
  const io = summarizeIo(events, 0, 50, roots);
  assert.equal(io.bytes, 140);
  assert.equal(io.graphBytes, 100);
  assert.equal(io.appDataBytes, 40);
  assert.equal(io.filesCreated, 1);
  assert.equal(io.renames, 1);
  assert.equal(io.fileSyncs, 1);
  assert.equal(io.dirSyncs, 1);
  assert.equal(io.syncCalls, 2);
  assert.equal(io.failed, 1, "a failed syscall is counted as failed, not as work");
  assert.equal(summarizeIo(events, 50, 100, roots).bytes, 1, "window end is exclusive, start inclusive");
});

test("parseNdjson skips a torn final line", () => {
  assert.deepEqual(parseNdjson('{"a":1}\n{"b":2}\n{"c":'), [{ a: 1 }, { b: 2 }]);
});

test("the events adapter turns the candidate's bench file into neutral records", () => {
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "ab-ev-")), "events.ndjson");
  fs.writeFileSync(file, [
    { ev: "save_begin", t: 1000, key: "k1" },
    { ev: "published", t: 1040, key: "k1", path: "pages/A.md", version: 3, bytes_len: 20, text: "- qzxabc12\n" },
    { ev: "draft_durable", t: 1100, path: "pages/A.md", text: "- qzxabc123\n" },
    { ev: "mail_parse", t: 1200, parse_us: 85 },
    { ev: "unfreeze", t: 1300 },
    { ev: "launch_recovered", t: 1400, drafts: 20 },
  ].map((o) => JSON.stringify(o)).join("\n") + "\n");
  const adapter = new Adapter("events", file);
  assert.deepEqual(adapter.env(), { TINE_BENCH_EVENTS: file });
  assert.deepEqual(new Adapter("ipc", file).env(), {});
  const probe = { calls: [{ cmd: "page_submit", t0: 990, t1: 1000, ok: true, runs: [] }] };
  // collect() goes through the page; exercise the same path with a stub browser
  const browser = { execute: async () => probe };
  return adapter.collect(browser).then((r) => {
    assert.equal(r.publishes.length, 1);
    assert.equal(r.publishes[0].t, 1040);
    assert.deepEqual(r.saveSpans, [{ t0: 1000, t1: 1040 }]);
    assert.equal(r.drafts[0].t, 1100);
    assert.equal(r.extra.mailParse[0].parse_us, 85);
    assert.equal(r.extra.unfreeze.length, 1);
    assert.equal(r.extra.launchRecovered[0].drafts, 20);
    assert.equal(r.rtt[0].cmd, "page_submit");
    assert.equal(Adapter.findPublish(r, "qzxabc12", 0).t, 1040);
    assert.equal(Adapter.findPublish(r, "qzxabc123", 0), null, "a publish that carried less does not match");
    assert.equal(Adapter.findDraft(r, "qzxabc123", 0).t, 1100);
  });
});

test("the ipc adapter reads the base's save_pages response as the publish signal", async () => {
  const calls = [
    { cmd: "save_pages", t0: 100, t1: 160, ok: true, runs: ["qzxa1"] },
    { cmd: "save_pages", t0: 200, t1: 230, ok: false, runs: ["qzxa12"] },
    { cmd: "store_draft", t0: 240, t1: 250, ok: true, runs: ["qzxa12"] },
    { cmd: "save_pages", t0: 300, t1: null, ok: null, runs: [] },
  ];
  const r = await new Adapter("ipc", "unused").collect({ execute: async () => ({ calls }) });
  assert.deepEqual(r.publishes.map((p) => p.t), [160], "a failed or unfinished save is not a publish");
  assert.equal(r.saveSpans.length, 2);
  assert.deepEqual(r.drafts.map((d) => d.t), [250]);
  assert.equal(r.extra.custodyComplete.length, 0, "arms without events still expose empty candidate-only streams");
  assert.throws(() => new Adapter("nope", "x"), /unknown publish adapter/);
});

test("every metric a scenario can emit is known to the registry and every scenario is named", () => {
  for (const id of DEFAULT_SCENARIOS) assert.ok(SCENARIOS[id], id);
  for (const [id, s] of Object.entries(SCENARIOS)) {
    assert.equal(typeof s.run, "function", id);
    assert.equal(typeof s.timeoutMs({ runs: 5, sessionMinutes: 1 }), "number", id);
  }
  for (const needed of ["launch.firstPageMs", "typing1.afterLastKeyMs", "typing1.burstKeyToPublishP95Ms", "unit.edit1.syncCalls", "unit.edit60.bytes", "delete.custodyCompleteMs"]) {
    assert.ok(METRICS[needed], `metric ${needed} missing from the registry`);
  }
  assert.equal(metricInfo("carry2.unfreezeMs").candidateOnly, true);
  assert.equal(metricInfo("ext.mailParseP95Us").candidateOnly, true);
  assert.notEqual(metricInfo("typing1.afterLastKeyMs").candidateOnly, true);
});

const haveGcc = spawnSync("gcc", ["--version"], { encoding: "utf8" }).status === 0;
test("iotrace sees a write, fsync, rename and unlink made by a thread created after attach", { skip: !haveGcc || process.platform !== "linux" || process.arch !== "x64" }, async (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "iotrace-"));
  const bin = path.join(dir, "iotrace");
  const cc = spawnSync("gcc", ["-O2", "-Wall", "-o", bin, path.join(HERE, "lib", "iotrace.c")], { encoding: "utf8" });
  assert.equal(cc.status, 0, cc.stderr);
  const watched = path.join(dir, "watched");
  fs.mkdirSync(watched);
  // The target idles, then writes 1000 bytes + fsync + rename + unlink from a worker thread.
  const worker = `
    const { Worker, isMainThread } = require("node:worker_threads");
    const fs = require("node:fs");
    if (isMainThread) { setTimeout(() => new Worker(__filename), 1500); setTimeout(() => process.exit(0), 3500); }
    else { const f = ${JSON.stringify(path.join(watched, "a.tmp"))}; const fd = fs.openSync(f, "w"); fs.writeSync(fd, Buffer.alloc(1000, 120)); fs.fsyncSync(fd); fs.closeSync(fd); fs.renameSync(f, ${JSON.stringify(path.join(watched, "a"))}); fs.unlinkSync(${JSON.stringify(path.join(watched, "a"))}); }`;
  const workerFile = path.join(dir, "worker.cjs");
  fs.writeFileSync(workerFile, worker);
  const target = spawn(process.execPath, [workerFile], { stdio: "inherit" });
  const out = path.join(dir, "trace.ndjson");
  const tracer = spawn(bin, [String(target.pid), out, watched], { stdio: ["ignore", "pipe", "inherit"] });
  t.after(() => { try { tracer.kill("SIGTERM"); } catch { /* gone */ } try { target.kill("SIGKILL"); } catch { /* gone */ } });
  const ready = await new Promise((resolve) => {
    const timer = setTimeout(() => resolve(false), 10000);
    tracer.stdout.on("data", (d) => { if (String(d).includes("ready")) { clearTimeout(timer); resolve(true); } });
    tracer.once("exit", () => { clearTimeout(timer); resolve(false); });
  });
  if (!ready) return t.skip("ptrace is not permitted in this environment");
  const exited = (child) => new Promise((resolve) => { if (child.exitCode !== null || child.signalCode !== null) resolve(); else child.once("exit", resolve); });
  await exited(target);
  await new Promise((resolve) => setTimeout(resolve, 300));
  tracer.kill("SIGTERM");
  await exited(tracer);
  const events = parseNdjson(fs.readFileSync(out, "utf8"));
  const io = summarizeIo(events, 0, Infinity, { graph: watched, appData: "/nonexistent" });
  assert.equal(io.graphBytes, 1000, JSON.stringify(io));
  assert.equal(io.fileSyncs, 1);
  assert.equal(io.renames, 1);
  assert.equal(io.unlinks, 1);
});

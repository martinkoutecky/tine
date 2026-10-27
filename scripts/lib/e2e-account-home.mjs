// Linux native fixtures use the production namespace and real kernel locks,
// but resolve the account home into this run, never the user's passwd home.
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const unavailable = /\[tine\] owner-(?:lock|endpoint)-unavailable\b/;

export function accountHomeFixture(run, artifacts = run, inherited = process.env) {
  if (process.platform !== "linux") return { env: inherited, verify() {} };
  fs.mkdirSync(run, { recursive: true });
  fs.mkdirSync(artifacts, { recursive: true });
  const reuse = inherited.TINE_E2E_ACCOUNT_SHIM && inherited.TINE_E2E_ACCOUNT_HOME;
  const home = reuse ? inherited.TINE_E2E_ACCOUNT_HOME : fs.mkdtempSync(path.join(run, "account-home-"));
  const shim = reuse ? inherited.TINE_E2E_ACCOUNT_SHIM : path.join(run, "account-home.so");
  if (!reuse) {
    fs.chmodSync(home, 0o700);
    execFileSync("rtk", ["proxy", "cc", "-shared", "-fPIC", "-Wall", "-Werror",
      path.join(import.meta.dirname, "e2e-account-home.c"), "-o", shim, "-ldl"]);
  }
  const env = { ...inherited, TINE_E2E_ACCOUNT_HOME: home, TINE_E2E_ACCOUNT_SHIM: shim,
    LD_PRELOAD: [shim, ...(inherited.LD_PRELOAD || "").split(/[:\s]+/)].filter(Boolean).filter((value, index, all) => all.indexOf(value) === index).join(":"),
    TINE_DEBUG: "1", TINE_E2E_DEBUG_DIR: path.resolve(artifacts) };
  const observed = new Map();
  const stderrLogs = new Set();
  const testedCommit = inherited.TINE_SOURCE_REVISION || execFileSync("rtk", ["git", "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  function sample() {
    const owners = path.join(home, ".local/state/tine/owners");
    if (!fs.existsSync(owners)) return [];
    const locks = fs.readFileSync("/proc/locks", "utf8").split("\n").map(line => line.trim().split(/\s+/));
    const held = [];
    for (const name of fs.readdirSync(owners).filter(name => name.endsWith(".lock"))) {
      const file = path.join(owners, name);
      const description = fs.readFileSync(file, "utf8");
      const pid = description.match(/^pid (\d+)$/m)?.[1];
      if (!pid || !/^owner app$/m.test(description)) continue;
      const inode = String(fs.statSync(file).ino);
      if (!locks.some(fields => fields[3] === "WRITE" && fields[4] === pid && fields[5]?.endsWith(`:${inode}`))) continue;
      // Stale descriptive bytes alone are never accepted as lease evidence.
      held.push({ file, pid: Number(pid), inode });
      observed.set(`${pid}:${inode}`, held.at(-1));
      // Older journeys keep their driver log outside E2E_ARTIFACT_DIR. Follow
      // only stderr of the proven fixture owner, never arbitrary /proc output.
      try {
        const stderr = fs.readlinkSync(`/proc/${pid}/fd/2`);
        if (path.isAbsolute(stderr) && fs.statSync(stderr).isFile()) stderrLogs.add(stderr);
      } catch { /* The owner may have exited since the kernel snapshot. */ }
    }
    return held;
  }
  function verify({ requireLease = true } = {}) {
    sample();
    const bad = [];
    function scan(dir) {
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const file = path.join(dir, entry.name);
        if (entry.isDirectory() && file !== home) scan(file);
        else if (entry.isFile() && entry.name.endsWith(".log") && unavailable.test(fs.readFileSync(file, "utf8"))) bad.push(file);
      }
    }
    scan(artifacts);
    for (const file of stderrLogs) {
      if (fs.existsSync(file) && unavailable.test(fs.readFileSync(file, "utf8")) && !bad.includes(file)) bad.push(file);
    }
    const fallback = inherited.TINE_E2E_EXPECT_OWNER_UNAVAILABLE;
    fs.writeFileSync(path.join(artifacts, "owner-lease.json"), JSON.stringify({ home, observed: [...observed.values()], stderrLogs: [...stderrLogs], fallback: fallback || null, unavailableLogs: bad }, null, 2) + "\n");
    const failure = !fallback && (bad.length ? `Unexpected owner arbitration fallback: ${bad.join(", ")}`
      : requireLease && !observed.size ? `No app-held kernel owner lease observed in ${home}` : null);
    if (failure) {
      fs.writeFileSync(path.join(artifacts, "owner-fixture-failure.json"), JSON.stringify({ testedCommit,
        journey: process.argv[1], expected: "App holds a real owner lease with no arbitration fallback",
        observed: failure, logs: bad, classification: "harness" }, null, 2) + "\n");
      throw new Error(failure);
    }
  }
  const timer = setInterval(sample, 100);
  timer.unref();
  return { env, sample, verify, close() { clearInterval(timer); } };
}

// Direct journeys install after seeding (some recreate their entire temp dir).
// The suite runner supplies the same fixture to every app and CLI child.
export function installAccountHome(run, artifacts = run) {
  const fixture = accountHomeFixture(run, artifacts);
  Object.assign(process.env, fixture.env);
  process.once("exit", () => {
    fixture.close?.();
    try { fixture.verify(); }
    catch (error) { console.error(error.message); process.exitCode = 1; }
  });
  return fixture;
}

export async function assertAppLease(fixture) {
  if (!fixture.sample || fixture.env.TINE_E2E_EXPECT_OWNER_UNAVAILABLE) return;
  const deadline = Date.now() + 20_000;
  while (!fixture.sample().length) {
    fixture.verify({ requireLease: false });
    if (Date.now() >= deadline) throw new Error("The opened graph has no app-held kernel owner lease");
    await sleep(100);
  }
  fixture.verify();
  console.log("Owner lease asserted: app holds the kernel lock");
}

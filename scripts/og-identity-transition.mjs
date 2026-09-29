#!/usr/bin/env node
// Native differential for the master -> og transition (docs/app-identity.md), Linux.
// Every app-data byte comes from the builds under test, never from this script:
//
//   master   A released Tine (MASTER_APP) opens a copy of GRAPH_SRC in a fresh
//            HOME; the user toggles the theme and opens a page, so master's own
//            writers fill its app-data dir.
//   seed     An experiment og build (OG_EXPERIMENT_APP) starts with NO graph
//            argument on a copy of that HOME: it must show master's graph,
//            theme and page (no Welcome, scope D/E), leave master's dir
//            byte-identical, and copy no master-only artifact.
//   inplace  A release-identity og build (OG_RELEASE_APP) starts on another
//            copy: same visible config (B/E); every master-only artifact and
//            every master backup snapshot byte-identical.
//   rollback MASTER_APP starts again on the dir og just used (C).
//
// Usage: MASTER_APP=… OG_EXPERIMENT_APP=… OG_RELEASE_APP=… GRAPH_SRC=… \
//        node scripts/og-identity-transition.mjs [--phases=seed,inplace,rollback]
// With a pre-change og build as OG_EXPERIMENT_APP the `seed` phase must FAIL.
import { spawn } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { IDENTITIES } from "./lib/app-identity.mjs";

const need = (name) => {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return path.resolve(value);
};
const MASTER_APP = need("MASTER_APP");
const GRAPH_SRC = need("GRAPH_SRC");
const PHASES = (process.argv.find((arg) => arg.startsWith("--phases="))?.slice(9)
  ?? "seed,inplace,rollback").split(",");
const TMP = process.env.E2E_TMP_DIR || `/tmp/tine-identity-transition-${process.pid}`;
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4670);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4671);
const RELEASE_ID = IDENTITIES.release.identifier;
const EXPERIMENT_ID = IDENTITIES.experiment.identifier;
const OPENED_PAGE = process.env.OPENED_PAGE || "Sheets demo";

// App-data entries the released Tine writes and og has no reader for
// (docs/app-identity.md, inventory): never copied, never changed.
const MASTER_ONLY = [
  "direct-files-projections", "direct-move-recovery", "conflict-capsules", "mediakeys",
  "hsts-storage.sqlite",
];
const isMasterOnly = (rel) => MASTER_ONLY.some((entry) => rel === entry || rel.startsWith(`${entry}/`));
// Master's backup snapshots (schema 3): og must neither copy nor prune them.
const isMasterSnapshot = (rel) => /^backups\/[^/]+\/[^/]+\//.test(rel);

function hashTree(root) {
  const out = {};
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile()) {
        out[path.relative(root, full)] = crypto.createHash("sha256").update(fs.readFileSync(full)).digest("hex");
      }
    }
  };
  walk(root);
  return out;
}
const changedKeys = (before, after) =>
  [...new Set([...Object.keys(before), ...Object.keys(after)])].filter((key) => before[key] !== after[key]).sort();

function homeEnv(home, extra) {
  return {
    PATH: process.env.PATH,
    HOME: home,
    XDG_DATA_HOME: `${home}/.local/share`,
    XDG_CONFIG_HOME: `${home}/.config`,
    XDG_CACHE_HOME: `${home}/.cache`,
    XDG_RUNTIME_DIR: `${home}/run`,
    DISPLAY: process.env.DISPLAY,
    XAUTHORITY: process.env.XAUTHORITY,
    LANG: "C.UTF-8",
    LIBGL_ALWAYS_SOFTWARE: "1",
    GDK_BACKEND: "x11",
    WEBKIT_DISABLE_DMABUF_RENDERER: "1",
    WEBKIT_DISABLE_COMPOSITING_MODE: "1",
    ...extra,
  };
}

/** Start `app` under WebDriver with `home` as its HOME, run `body`, then kill it. */
async function withApp(label, app, home, extra, body) {
  fs.mkdirSync(`${home}/run`, { recursive: true, mode: 0o700 });
  const log = fs.openSync(`${TMP}/${label}-driver.log`, "w");
  const td = spawn(TD, webdriverServerArgs(DRIVER_PORT, NATIVE_PORT, "/usr/bin/WebKitWebDriver"), {
    env: homeEnv(home, extra), stdio: ["ignore", log, log], detached: true,
  });
  await sleep(2500);
  let browser;
  try {
    browser = await remote({
      hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
      connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: tauriCapabilities(app, label),
    });
    // Welcome or a routed page: whichever the startup graph load settles on.
    await browser.waitUntil(async () => browser.execute(() =>
      Boolean(document.querySelector(".welcome-overlay, h1.page-title"))), {
      timeout: 45_000, interval: 250, timeoutMsg: `${label}: neither Welcome nor a page rendered`,
    });
    await sleep(2000);
    return await body(browser);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    await sleep(2000);
  }
}

const visibleConfig = (browser) => browser.execute(() => ({
  welcome: Boolean(document.querySelector(".welcome-overlay")),
  theme: document.documentElement.getAttribute("data-theme"),
  title: (document.querySelector("h1.page-title")?.textContent ?? "").trim().normalize("NFC"),
}));

function copyHome(from, to) {
  fs.rmSync(to, { recursive: true, force: true });
  fs.cpSync(from, to, { recursive: true, verbatimSymlinks: true });
}

const report = { phases: {}, failures: [] };
const expect = (phase, ok, message) => { if (!ok) report.failures.push(`${phase}: ${message}`); };

fs.rmSync(TMP, { recursive: true, force: true });
fs.mkdirSync(TMP, { recursive: true });
const graph = `${TMP}/graph`;
fs.cpSync(GRAPH_SRC, graph, { recursive: true });
const masterHome = `${TMP}/master-home`;

// --- master writes its own app-data dir -------------------------------------
const master = await withApp("master", MASTER_APP, masterHome, { TINE_GRAPH: graph }, async (browser) => {
  const initial = await visibleConfig(browser);
  await openPageByName(browser, OPENED_PAGE);
  await browser.execute(() => document.activeElement instanceof HTMLElement && document.activeElement.blur());
  await browser.keys(["t"]);
  await browser.keys(["t"]);
  await browser.waitUntil(async () => (await visibleConfig(browser)).theme !== initial.theme, {
    timeout: 5000, timeoutMsg: "master: `t t` did not change the theme",
  });
  await sleep(4000); // debounced session/workspace + settings writes land
  return { initial, acted: await visibleConfig(browser) };
});
report.phases.master = master;
expect("master", master.acted.title === OPENED_PAGE, `master did not route to ${OPENED_PAGE}`);
const masterData = `${masterHome}/.local/share/${RELEASE_ID}`;
const pristine = `${TMP}/master-home.pristine`;
copyHome(masterHome, pristine);
const masterTree = hashTree(masterData);
report.phases.master.files = Object.keys(masterTree).sort();
expect("master", Object.keys(masterTree).some(isMasterOnly), "fixture has no master-only artifact to protect");
expect("master", Object.keys(masterTree).some(isMasterSnapshot), "fixture has no master backup snapshot");

const matchesMaster = (phase, seen) => {
  expect(phase, !seen.welcome, "the Welcome screen is showing");
  expect(phase, seen.theme === master.acted.theme, `theme ${seen.theme} != master's ${master.acted.theme}`);
  expect(phase, seen.title === OPENED_PAGE, `routed page ${JSON.stringify(seen.title)} != master's ${OPENED_PAGE}`);
};

if (PHASES.includes("seed")) {
  const home = `${TMP}/seed-home`;
  copyHome(pristine, home);
  const seen = await withApp("seed", need("OG_EXPERIMENT_APP"), home, {}, visibleConfig);
  const masterChanged = changedKeys(masterTree, hashTree(`${home}/.local/share/${RELEASE_ID}`));
  // og writes its own backups/webview files; a COPY is master's path with master's
  // bytes. WebKit creates an empty hsts-storage.sqlite byte-identical to master's
  // (a pre-change og build, which copies nothing, produces the same bytes).
  const ownTree = hashTree(`${home}/.local/share/${EXPERIMENT_ID}`);
  const copiedMasterOnly = Object.keys(ownTree)
    .filter((rel) => rel !== "hsts-storage.sqlite")
    .filter((rel) => (isMasterOnly(rel) || isMasterSnapshot(rel)) && ownTree[rel] === masterTree[rel]);
  report.phases.seed = { seen, masterChanged, copiedMasterOnly };
  matchesMaster("seed", seen);
  expect("seed", masterChanged.length === 0, `master's dir changed: ${masterChanged.join(", ")}`);
  expect("seed", copiedMasterOnly.length === 0, `master-only artifacts copied: ${copiedMasterOnly.join(", ")}`);
}

if (PHASES.includes("inplace") || PHASES.includes("rollback")) {
  const home = `${TMP}/inplace-home`;
  copyHome(pristine, home);
  const seen = await withApp("inplace", need("OG_RELEASE_APP"), home, {}, visibleConfig);
  const changed = changedKeys(masterTree, hashTree(`${home}/.local/share/${RELEASE_ID}`));
  const protectedChanged = changed.filter((rel) =>
    isMasterOnly(rel) || (isMasterSnapshot(rel) && rel in masterTree));
  report.phases.inplace = { seen, changed, protectedChanged };
  matchesMaster("inplace", seen);
  expect("inplace", protectedChanged.length === 0, `master-only artifacts changed: ${protectedChanged.join(", ")}`);
  expect("inplace", !fs.existsSync(`${home}/.local/share/${EXPERIMENT_ID}`), "a release build created an experiment dir");

  if (PHASES.includes("rollback")) {
    const back = await withApp("rollback", MASTER_APP, home, {}, visibleConfig);
    report.phases.rollback = { seen: back };
    matchesMaster("rollback", back);
  }
}

fs.writeFileSync(`${TMP}/report.json`, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify(report, null, 2));
process.exit(report.failures.length ? 1 : 0);

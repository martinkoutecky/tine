#!/usr/bin/env node

// Focused Windows proof for GH #292 and #396. This is a real WebView2 + Tauri + NTFS
// journey, including the native confirmation dialogs and the production
// Settings action that invokes the managed-storage commands.
import crypto from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  selectWebdriverWindowWithSelector,
  startWebdriverApplication,
  stopWebdriverApplication,
  tauriCapabilities,
  webdriverServerArgs,
} from "./e2e-capabilities.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

if (process.platform !== "win32") throw new Error("Windows managed-storage smoke must run on Windows");
const APP = process.env.TINE_APP;
if (!APP || !fs.existsSync(APP)) throw new Error("HARNESS UNAVAILABLE: Windows managed-storage smoke requires TINE_APP");
const BASELINE_APP = process.env.TINE_BASELINE_APP || APP;
if (!fs.existsSync(BASELINE_APP)) throw new Error("HARNESS UNAVAILABLE: TINE_BASELINE_APP does not exist");

const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4444);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4445);
const PAGE_COUNT = Number(process.env.E2E_MANAGED_PAGE_COUNT || 13_000);
const BLOCKS_PER_PAGE = Number(process.env.E2E_MANAGED_BLOCKS_PER_PAGE || 10);
const TOTAL_FILE_COUNT = Number(process.env.E2E_MANAGED_TOTAL_FILE_COUNT || 25_890);
const ASSET_LOGICAL_BYTES = Number(process.env.E2E_MANAGED_ASSET_LOGICAL_BYTES || 25_200_000_000);
const ACTIVATION_TIMEOUT_MS = Number(process.env.E2E_MANAGED_ACTIVATION_TIMEOUT_MS || 30 * 60_000);
// The first query owns reporter-scale cold projection readiness. Keep its
// finite ceiling distinct from the 30 s navigation requirement measured after
// readiness, so a slow or stuck index cannot be hidden as ordinary page-switch
// performance.
const COLD_INDEX_TIMEOUT_MS = Number(process.env.E2E_MANAGED_COLD_INDEX_TIMEOUT_MS || 5 * 60_000);
const CURRENT_ONLY = process.env.E2E_MANAGED_CURRENT_ONLY === "true";
const canonicalExecutable = (value) => fs.realpathSync(value).toLocaleLowerCase("en-US");
const candidateExecutable = canonicalExecutable(APP);
const activationExecutable = canonicalExecutable(BASELINE_APP);
if (CURRENT_ONLY !== (candidateExecutable === activationExecutable)) {
  throw new Error(
    `managed mode/executable mismatch: currentOnly=${CURRENT_ONLY} candidate=${candidateExecutable} activation=${activationExecutable}`,
  );
}
for (const [name, value, minimum] of [
  ["E2E_MANAGED_PAGE_COUNT", PAGE_COUNT, 2],
  ["E2E_MANAGED_BLOCKS_PER_PAGE", BLOCKS_PER_PAGE, 1],
  ["E2E_MANAGED_TOTAL_FILE_COUNT", TOTAL_FILE_COUNT, PAGE_COUNT + 3],
  ["E2E_MANAGED_ASSET_LOGICAL_BYTES", ASSET_LOGICAL_BYTES, 0],
  ["E2E_MANAGED_ACTIVATION_TIMEOUT_MS", ACTIVATION_TIMEOUT_MS, 60_000],
  ["E2E_MANAGED_COLD_INDEX_TIMEOUT_MS", COLD_INDEX_TIMEOUT_MS, 30_000],
]) {
  if (!Number.isSafeInteger(value) || value < minimum) throw new Error(`invalid ${name} ${value}`);
}

const root = path.join(os.tmpdir(), `tine-windows-managed-${process.pid}`);
const graph = path.join(root, "graph-研究");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts"));
const debugLog = path.join(artifacts, "tine-debug.log");
const nestedTitle = "Résumé 日本語";
const nestedMarker = "WINDOWS_MANAGED_NESTED_UTF_MARKER";
const ordinaryTitle = "Windows Managed 00002";
const nestedFile = path.join(graph, "pages", "研究", `${nestedTitle}.md`);
const now = new Date();
const journalStem = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
const journalMarker = "WINDOWS_MANAGED_JOURNAL_MARKER";

fs.rmSync(root, { recursive: true, force: true });
for (const dir of ["pages", "pages/研究", "journals", "logseq", "assets/层级/二"]) {
  fs.mkdirSync(path.join(graph, ...dir.split("/")), { recursive: true });
}
fs.mkdirSync(artifacts, { recursive: true });
fs.writeFileSync(path.join(graph, "logseq", "config.edn"), '{:preferred-format "Markdown"}\n');
fs.writeFileSync(path.join(graph, "journals", `${journalStem}.md`), `- ${journalMarker}\n`);
const pageBody = (label, referenceTarget) => Array.from(
  { length: BLOCKS_PER_PAGE },
  (_, block) => `- ${label} block ${block + 1} references [[${referenceTarget}]] and #[[windows-managed]]`,
).join("\n") + "\n";
fs.writeFileSync(nestedFile, pageBody(nestedMarker, ordinaryTitle));
for (let index = 1; index < PAGE_COUNT; index += 1) {
  const bucket = path.join(graph, "pages", `bucket-${String(index % 37).padStart(2, "0")}`);
  fs.mkdirSync(bucket, { recursive: true });
  const stem = `Windows Managed ${String(index).padStart(5, "0")}`;
  const body = index === 1
    // Parseable Markdown whose source spans deliberately do not structurally
    // round-trip. Managed activation must retain the exact bytes read-only,
    // not reject the entire reporter-shaped graph (#292).
    ? "- root\r  ```\r  - fake\r  ```"
    // Keep graph-wide reference indexes representative without manufacturing
    // 120,000 backlinks to the one page the timing journey opens. That shape
    // measures long-backlink-query monopolization, not ordinary page switching.
    : pageBody(
      `fixture page ${index}`,
      `Windows Managed ${String(index + 1 < PAGE_COUNT ? index + 1 : 1).padStart(5, "0")}`,
    );
  fs.writeFileSync(path.join(bucket, `${stem}.md`), body);
}

// Match the reporter's two independent filesystem axes without transferring
// or allocating a 24.7 GiB Actions artifact: many ignored files exercise the
// graph walk, while one sparse file carries the large logical-byte shape.
const sparseAsset = path.join(graph, "assets", "reporter-shape-sparse.bin");
fs.closeSync(fs.openSync(sparseAsset, "w"));
const markedSparse = spawnSync("fsutil.exe", ["sparse", "setflag", sparseAsset], { encoding: "utf8" });
if (markedSparse.status !== 0) {
  throw new Error(`could not mark reporter-shape asset sparse: ${String(markedSparse.stderr || markedSparse.stdout).trim()}`);
}
const sparseHandle = fs.openSync(sparseAsset, "r+");
try {
  fs.ftruncateSync(sparseHandle, ASSET_LOGICAL_BYTES);
} finally {
  fs.closeSync(sparseHandle);
}
const sparseRange = spawnSync(
  "fsutil.exe",
  ["sparse", "setrange", sparseAsset, "0", String(ASSET_LOGICAL_BYTES)],
  { encoding: "utf8" },
);
if (sparseRange.status !== 0) {
  throw new Error(`could not retain compact reporter-shape asset: ${String(sparseRange.stderr || sparseRange.stdout).trim()}`);
}
const sourceFileCount = PAGE_COUNT + 1; // pages plus today's journal
const fixedNonSourceFiles = 2; // config.edn plus the sparse asset
const fillerAssetCount = TOTAL_FILE_COUNT - sourceFileCount - fixedNonSourceFiles;
for (let index = 0; index < fillerAssetCount; index += 1) {
  const bucket = path.join(graph, "assets", `asset-bucket-${String(index % 281).padStart(3, "0")}`);
  fs.mkdirSync(bucket, { recursive: true });
  fs.writeFileSync(path.join(bucket, `asset-${String(index).padStart(5, "0")}.bin`), "fixture\n");
}

function sourceSnapshot() {
  const snapshot = new Map();
  const visit = (directory) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(absolute);
      } else if (/\.(md|org)$/i.test(entry.name)) {
        const relative = path.relative(graph, absolute).split(path.sep).join("/");
        snapshot.set(relative, crypto.createHash("sha256").update(fs.readFileSync(absolute)).digest("hex"));
      }
    }
  };
  visit(graph);
  return snapshot;
}

function graphInventory() {
  const metrics = {
    totalFiles: 0,
    totalLogicalBytes: 0,
    sourceFiles: 0,
    sourceBytes: 0,
    blocks: 0,
    ignoredFiles: 0,
    ignoredLogicalBytes: 0,
  };
  const visit = (directory) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(absolute);
        continue;
      }
      if (!entry.isFile()) continue;
      const stat = fs.statSync(absolute);
      metrics.totalFiles += 1;
      metrics.totalLogicalBytes += stat.size;
      if (/\.(md|org)$/i.test(entry.name)) {
        const source = fs.readFileSync(absolute);
        metrics.sourceFiles += 1;
        metrics.sourceBytes += source.length;
        metrics.blocks += source
          .toString("utf8")
          .split(/\r?\n/)
          .filter((line) => line.startsWith("- ") || /^\*+ /.test(line))
          .length;
      } else {
        metrics.ignoredFiles += 1;
        metrics.ignoredLogicalBytes += stat.size;
      }
    }
  };
  visit(graph);
  return metrics;
}

function assertSameSource(before, label) {
  const after = sourceSnapshot();
  if (after.size !== before.size) throw new Error(`${label} changed source file count: ${before.size} -> ${after.size}`);
  for (const [relative, digest] of before) {
    if (after.get(relative) !== digest) throw new Error(`${label} changed source bytes for ${relative}`);
  }
}

function assertOriginalSourcesUnchanged(before, label) {
  const after = sourceSnapshot();
  for (const [relative, digest] of before) {
    if (after.get(relative) !== digest) throw new Error(`${label} changed original source bytes for ${relative}`);
  }
}

async function bodyText() {
  return browser.$("body").getText();
}

async function waitForBody(text, timeout, label) {
  await browser.waitUntil(async () => (await bodyText()).includes(text), {
    timeout,
    timeoutMsg: `${label} was not visible: ${JSON.stringify(text)}`,
  });
}

async function buttonContaining(text) {
  for (const button of await browser.$$("button")) {
    if ((await button.getText()).includes(text)) return button;
  }
  return undefined;
}

async function clickButton(text) {
  let button;
  await browser.waitUntil(async () => {
    button = await buttonContaining(text);
    return Boolean(button);
  }, { timeout: 15_000, timeoutMsg: `button was not visible: ${text}` });
  await button.click();
}

async function clickButtonAndConfirm(text) {
  // WebDriver cannot address native Windows dialogs. Tine owns the foreground
  // window in this isolated runner, so use the mechanism already proven to
  // cross v0.6.94's native MessageBox instead of trying to infer its localized
  // accessibility tree from the WebView's parent window.
  const confirmer = spawn("powershell.exe", [
    "-NoProfile",
    "-NonInteractive",
    "-Command",
    "Start-Sleep -Milliseconds 750; Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')",
  ], { windowsHide: true, stdio: "ignore" });
  const completed = new Promise((resolve) => {
    confirmer.once("exit", (code) => resolve(code));
    confirmer.once("error", () => resolve(-1));
  });
  await clickButton(text);
  const exitCode = await completed;
  if (exitCode !== 0) throw new Error(`native confirmation helper failed for ${text}: exit ${exitCode}`);
}

async function openManagedSettings(expectedActions) {
  const actions = Array.isArray(expectedActions) ? expectedActions : [expectedActions];
  const settings = await browser.$('button[title^="Settings"]');
  await settings.waitForExist({ timeout: 15_000 });
  await settings.click();
  await browser.$(".settings-modal").waitForExist({ timeout: 15_000 });
  for (const item of await browser.$$(".settings-nav-item")) {
    if ((await item.getText()).trim() === "Backups & recovery") {
      await item.click();
      break;
    }
  }
  await waitForBody("Storage & sync", 15_000, "storage settings");
  let availableAction;
  await browser.waitUntil(async () => {
    for (const action of actions) {
      const button = await buttonContaining(action);
      if (button && await button.isDisplayed()) {
        availableAction = action;
        return true;
      }
    }
    const experimental = await browser.$(".settings-experimental .settings-advanced-toggle");
    if (await experimental.isExisting() && await experimental.getAttribute("aria-expanded") !== "true") {
      await experimental.click();
    }
    return false;
  }, { timeout: 15_000, timeoutMsg: `managed storage action did not expand: ${actions.join(" or ")}` });
  return availableAction;
}

async function closeSettings() {
  const close = await browser.$(".settings-pane-head .icon-btn:not(.settings-maximize)");
  await close.waitForClickable({ timeout: 15_000 });
  await close.click();
  await browser.$(".settings-modal").waitForExist({ reverse: true, timeout: 15_000 });
}

async function waitForActivation() {
  const deadline = Date.now() + ACTIVATION_TIMEOUT_MS;
  let last = "";
  while (Date.now() < deadline) {
    last = await bodyText();
    if (last.includes("Tine-managed storage active")) return;
    if (last.includes("Setup can be retried.") || last.includes("This graph cannot use Tine-managed storage.")) {
      throw new Error(`managed activation returned non-active status: ${last.slice(-1000)}`);
    }
    await sleep(500);
  }
  throw new Error(`managed activation timed out; last body=${last.slice(-1000)}`);
}

async function openPage(title, { expectedMarker = nestedMarker, requireHeading = true, timeout = 30_000 } = {}) {
  // WebView2 attachment does not guarantee native keyboard focus, so this
  // journey enters the switcher through the visible Search control rather than
  // Ctrl+K; fixture navigation is not a shortcut assertion. Everything else --
  // excluding block hits (a block whose text contained the page title is what
  // broke run 33350869828), exact name matching, and activating in ONE round
  // trip so no row handle outlives the switcher's re-render -- is now the
  // shared contract in scripts/lib/e2e-navigation.mjs.
  await openPageByName(browser, title, { entry: "button", timeout });
  if (requireHeading) {
    const heading = await browser.$("h1.page-title");
    await heading.waitForExist({ timeout: 30_000 });
    await browser.waitUntil(async () => (await heading.getText()).includes(title), {
      timeout: 30_000,
      timeoutMsg: `page heading did not switch to ${title}`,
    });
  }
  if (expectedMarker) {
    await waitForBody(expectedMarker, 30_000, "nested UTF page after activation");
  }
}

function timingReceipt(samples) {
  const ordered = [...samples].sort((a, b) => a - b);
  return {
    samples: ordered.length,
    interaction: "open visible switcher, type exact title, choose result, wait for heading and page marker",
    minMs: ordered[0],
    p50Ms: ordered[Math.floor(ordered.length / 2)],
    maxMs: ordered[ordered.length - 1],
  };
}

async function measurePageSwitches(rounds = 8) {
  const ordinaryMarker = "fixture page 2 block 1";
  const samples = [];
  for (let round = 0; round < rounds; round += 1) {
    // Each measurement is a real transition: callers begin on the nested page,
    // so visit the ordinary page first and alternate from there.
    const nested = round % 2 === 1;
    const started = Date.now();
    await openPage(nested ? nestedTitle : ordinaryTitle, {
      expectedMarker: nested ? nestedMarker : ordinaryMarker,
    });
    samples.push(Date.now() - started);
  }
  return timingReceipt(samples);
}

async function createManagedPageAndAttemptEdit() {
  const title = "升级恢复 中文页";
  const marker = "WINDOWS_MANAGED_PREVIOUS_PATCH_EDIT";
  const newPage = await browser.$("button.new-page-btn");
  await newPage.waitForClickable({ timeout: 15_000 });
  await newPage.click();
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 15_000 });
  await input.setValue(title);
  let createRow;
  await browser.waitUntil(async () => {
    for (const row of await browser.$$(".switcher-row")) {
      if ((await row.getText()).includes(`Create page: ${title}`)) {
        createRow = row;
        return true;
      }
    }
    return false;
  }, { timeout: 30_000, timeoutMsg: `Create page result was not visible for ${title}` });
  await createRow.click();
  const heading = await browser.$("h1.page-title");
  await heading.waitForExist({ timeout: 15_000 });
  let editor = await browser.$(".page-blocks textarea.block-editor, textarea.block-editor");
  if (!await editor.isExisting()) {
    const target = await browser.$(".page-trailing-block-target");
    await target.waitForExist({ timeout: 15_000 });
    await target.click();
    editor = await browser.$(".page-blocks textarea.block-editor, textarea.block-editor");
    await editor.waitForExist({ timeout: 15_000 });
  }
  await editor.addValue(marker);
  await heading.click();
  // v0.6.94 could reject this save (#292/#366). That refusal is part of the
  // reporter's predecessor state, not a reason to abort the upgrade fixture.
  await sleep(3_000);
  return {
    title,
    marker,
    bodyAfterSaveAttempt: (await bodyText()).slice(-2_000),
  };
}

const before = sourceSnapshot();
const inventory = graphInventory();
if (inventory.totalFiles !== TOTAL_FILE_COUNT) {
  throw new Error(`fixture file-count mismatch: expected ${TOTAL_FILE_COUNT}, got ${inventory.totalFiles}`);
}
if (inventory.sourceFiles !== PAGE_COUNT + 1) {
  throw new Error(`fixture source-count mismatch: expected ${PAGE_COUNT + 1}, got ${inventory.sourceFiles}`);
}
if (inventory.blocks < PAGE_COUNT * BLOCKS_PER_PAGE - BLOCKS_PER_PAGE) {
  throw new Error(`fixture block-count mismatch: expected reporter scale, got ${inventory.blocks}`);
}
const env = {
  ...process.env,
  TINE_GRAPH: graph,
  TINE_DEBUG: "1",
  TINE_DEBUG_LOG: debugLog,
  TINE_ACTIVATION_TRACE: "1",
  APPDATA: path.join(root, "appdata"),
  LOCALAPPDATA: path.join(root, "localappdata"),
};
let browser;
let webviewTarget;
let driver;
let driverLog;

async function startJourney(application, label) {
  const launchEnv = {
    ...env,
    TINE_E2E_APPLICATION_STDOUT_LOG: path.join(artifacts, `application-${label}-stdout.log`),
    TINE_E2E_APPLICATION_STDERR_LOG: path.join(artifacts, `application-${label}-stderr.log`),
  };
  webviewTarget = await startWebdriverApplication(application, launchEnv, NATIVE_PORT);
  driverLog = fs.openSync(path.join(artifacts, `tauri-driver-${label}.log`), "w");
  driver = spawn(TD, webdriverServerArgs(DRIVER_PORT), {
    env: webviewTarget.env,
    stdio: ["ignore", driverLog, driverLog],
  });
  await sleep(3000);
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    capabilities: tauriCapabilities(application, "default", process.platform, webviewTarget.debuggerAddress),
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
  });
  await selectWebdriverWindowWithSelector(browser, 'button[title^="Search (Ctrl+K)"]');
  await browser.$(".ls-block").waitForExist({ timeout: 180_000 });
}

async function stopJourney(forceApplicationFirst = false) {
  if (forceApplicationFirst) {
    stopWebdriverApplication(webviewTarget);
    webviewTarget = undefined;
  }
  try { await browser?.deleteSession(); } catch {}
  browser = undefined;
  if (driver?.pid) spawnSync("taskkill", ["/PID", String(driver.pid), "/T", "/F"], { stdio: "ignore" });
  driver = undefined;
  stopWebdriverApplication(webviewTarget);
  webviewTarget = undefined;
  if (driverLog !== undefined) {
    try { fs.closeSync(driverLog); } catch {}
    driverLog = undefined;
  }
}

const receipt = {
  schemaVersion: 1,
  scenario: "windows-managed-storage",
  pageCount: PAGE_COUNT,
  blocksPerPage: BLOCKS_PER_PAGE,
  graph: inventory,
  candidateOnly: CURRENT_ONLY,
  executables: {
    candidate: {
      path: candidateExecutable,
      sha256: crypto.createHash("sha256").update(fs.readFileSync(APP)).digest("hex"),
    },
    activation: {
      path: activationExecutable,
      sha256: crypto.createHash("sha256").update(fs.readFileSync(BASELINE_APP)).digest("hex"),
    },
  },
  graphPathKinds: ["nested", "unicode", "markdown", "parseable-non-roundtripping-markdown", "distributed-page-references", "many-assets", "sparse-large-asset"],
  milestones: {},
};
try {
  await startJourney(BASELINE_APP, "activation");
  const coldIndexStarted = Date.now();
  await openPage(nestedTitle, { timeout: COLD_INDEX_TIMEOUT_MS });
  receipt.milestones.directFilesColdReady = {
    elapsedMs: Date.now() - coldIndexStarted,
    maxMs: COLD_INDEX_TIMEOUT_MS,
  };
  receipt.milestones.directFilesOpened = true;
  receipt.milestones.directFilesPageSwitch = await measurePageSwitches();

  await openManagedSettings("Enable Tine-managed storage...");
  const activationStarted = Date.now();
  await clickButtonAndConfirm("Enable Tine-managed storage...");
  await waitForActivation();
  receipt.milestones.activationMs = Date.now() - activationStarted;
  assertSameSource(before, "managed activation");
  await closeSettings();
  if (CURRENT_ONLY) {
    // Candidate-only performance evidence must never inherit the historical
    // predecessor's tolerated blank-shell behavior.
    await openPage(nestedTitle);
    receipt.milestones.baselineManagedPageBodyVisible = true;
    receipt.milestones.managedPageSwitch = await measurePageSwitches();
    receipt.milestones.managedEdit = await createManagedPageAndAttemptEdit();
  } else {
    // v0.6.94 may publish the managed page shell while leaving its body blank.
    // That is predecessor evidence for #370, not a reason to abort before the
    // candidate gets a chance to recover the same private state.
    await openPage(nestedTitle, { expectedMarker: null, requireHeading: false });
    receipt.milestones.baselineManagedPageBodyVisible = (await bodyText()).includes(nestedMarker);
    if (receipt.milestones.baselineManagedPageBodyVisible) {
      receipt.milestones.managedPageSwitch = await measurePageSwitches();
      receipt.milestones.previousPatchEdit = await createManagedPageAndAttemptEdit();
    } else {
      receipt.milestones.previousPatchEdit = {
        attempted: false,
        reason: "v0.6.94 published a managed page shell without its body",
      };
    }
  }

  // GH #370 is an upgrade/reopen failure, not an activation failure. Kill the
  // actual baseline process without a graceful managed shutdown, then require
  // the candidate to recover the same private state and serve an ordinary page.
  await stopJourney(true);
  receipt.milestones.baselineForceClosed = true;
  const reopenStarted = Date.now();
  await startJourney(APP, "candidate-reopen");
  receipt.milestones.candidateReopenMs = Date.now() - reopenStarted;
  await openPage(nestedTitle);
  receipt.milestones.candidateReopenReadyMs = Date.now() - reopenStarted;
  receipt.milestones.candidateManagedPageOpened = true;
  receipt.milestones.reopenedManagedPageSwitch = await measurePageSwitches();

  const directFilesAction = await openManagedSettings([
    "Return to Direct files",
    "Open current files in Direct Files...",
  ]);
  await clickButtonAndConfirm(directFilesAction);
  await waitForBody("Enable Tine-managed storage...", 120_000, "Direct Files status after rollback");
  // The attempted v0.6.94 creation may finish before or during recovery. Its
  // new file is allowed; authority transitions may not alter any incumbent.
  assertOriginalSourcesUnchanged(before, "return to Direct Files");
  await closeSettings();
  await openPage(nestedTitle);
  receipt.milestones.directFilesRestored = true;
  if (fs.existsSync(debugLog)) {
    receipt.activationTrace = fs.readFileSync(debugLog, "utf8")
      .split(/\r?\n/)
      .filter((line) => line.includes("sparse-v2 activation"))
      .map((line) => line.replaceAll(graph, "<GRAPH>"))
      .slice(-250);
  }
  receipt.result = "pass";
  fs.writeFileSync(path.join(artifacts, "windows-managed-storage-receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(`PASS: Windows managed storage activation and Direct Files return: ${JSON.stringify(receipt)}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(artifacts, "failure.png")); } catch {}
  let lastBody = "";
  try { lastBody = (await bodyText()).slice(-4_000); } catch {}
  let activationTrace = [];
  try {
    activationTrace = fs.readFileSync(debugLog, "utf8")
      .split(/\r?\n/)
      .filter((line) => line.includes("sparse-v2 activation"))
      .map((line) => line.replaceAll(graph, "<GRAPH>"))
      .slice(-250);
  } catch {}
  const failure = {
    ...receipt,
    result: "fail",
    error: String(error).split("\n").slice(0, 8).join(" | "),
    lastBody,
    activationTrace,
  };
  fs.writeFileSync(path.join(artifacts, "failure-capsule.json"), `${JSON.stringify(failure, null, 2)}\n`);
  throw error;
} finally {
  await stopJourney();
  if (process.env.CI === "true") spawnSync("taskkill", ["/IM", path.basename(APP), "/T", "/F"], { stdio: "ignore" });
}

#!/usr/bin/env node

// Linux real-app proof for sparse-v2's user-visible authority journey. The
// semantic oracles are rendered note text and ordinary Markdown files; this
// deliberately does not inspect private sparse state or storage layout.
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { waitForFileText as waitForPersistedFileText } from "./e2e-file-poll.mjs";
import { ensureDisplay } from "./lib/e2e-display.mjs";

await ensureDisplay();

if (process.platform !== "linux") throw new Error("sparse-v2 recovery native proof is Linux-only");
if (!process.env.TINE_APP) throw new Error("HARNESS UNAVAILABLE: sparse-v2 recovery requires the exact candidate in TINE_APP");

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = path.resolve(process.env.TINE_APP);
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const XDOTOOL = process.env.E2E_XDOTOOL || "xdotool";
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4624);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4625);
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "tine-sparse-v2-recovery-"));
const GRAPH = path.join(TMP, "graph");
const XDG = path.join(TMP, "xdg");
const ARTIFACTS = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts"));
const NORMAL_PAGE = "Welcome";
const NESTED_PAGE = "Résumé 日本語";
const NEW_PAGE = "Sparse V2 Recovery New";
const NORMAL_MARKER = "ordinary legacy page remains visible";
const NESTED_ORIGINAL = "nested UTF original content — café 日本語";
const NESTED_EDIT = "sparse v2 saved existing UTF page";
const NESTED_WINNER_A = "managed conflict winner A";
const NESTED_WINNER_B = "managed conflict winner B before Keep mine";
const NEW_EDIT = "sparse v2 saved newly created page";
const NEW_WINNER = "concurrent creator won the new page race";
const STANDARD_EDIT = "standard Markdown saved after rollback";
const JOURNAL_MARKER = "today canonical journal remains visible";
const NORMAL_FILE = path.join(GRAPH, "pages", `${NORMAL_PAGE}.md`);
const NESTED_FILE = path.join(GRAPH, "pages", "研究", "Résumé 日本語.md");
const NEW_FILE = path.join(GRAPH, "pages", `${NEW_PAGE}.md`);

if (!fs.existsSync(APP)) throw new Error(`HARNESS UNAVAILABLE: exact TINE_APP candidate is missing at ${APP}`);
fs.mkdirSync(ARTIFACTS, { recursive: true });
for (const dir of ["pages", "pages/研究", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(XDG, dir), { recursive: true });

const today = new Date();
const journalStem = `${today.getFullYear()}_${String(today.getMonth() + 1).padStart(2, "0")}_${String(today.getDate()).padStart(2, "0")}`;
const JOURNAL_FILE = path.join(GRAPH, "journals", `${journalStem}.md`);
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), '{:preferred-format "Markdown"}\n');
fs.writeFileSync(NORMAL_FILE, `- ${NORMAL_MARKER}\n`);
fs.writeFileSync(NESTED_FILE, `- ${NESTED_ORIGINAL}\n`);
fs.writeFileSync(JOURNAL_FILE, `- ${JOURNAL_MARKER}\n`);

// A persisted native frame gives the native close proof one real, visible GTK
// title-bar control. The profile is otherwise empty and remains the exact same
// one across both relaunches.
const APP_DATA = path.join(XDG, "data", "page.tine.Tine");
fs.mkdirSync(APP_DATA, { recursive: true });
fs.writeFileSync(path.join(APP_DATA, "tine-settings.json"), '{"native_window_frame":true}\n');

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(XDG, "data"),
  XDG_CONFIG_HOME: path.join(XDG, "config"),
  XDG_CACHE_HOME: path.join(XDG, "cache"),
  XDG_CONFIG_DIRS: process.env.XDG_CONFIG_DIRS || "/etc/xdg",
  XDG_DATA_DIRS: process.env.XDG_DATA_DIRS || "/usr/local/share:/usr/share",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const xdoEnv = process.env.E2E_XDOTOOL_LIB
  ? { ...env, LD_LIBRARY_PATH: process.env.E2E_XDOTOOL_LIB }
  : env;
const xdo = (...args) => execFileSync(XDOTOOL, args, { encoding: "utf8", env: xdoEnv }).trim();

function gitRevision() {
  const result = spawnSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" });
  return result.status === 0 ? result.stdout.trim() : "unavailable";
}

function processAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

async function waitFor(predicate, timeoutMs, message) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await predicate();
    if (value) return value;
    await sleep(50);
  }
  throw new Error(message);
}

function geometry(id) {
  const raw = execFileSync("xwininfo", ["-id", id], { encoding: "utf8", env });
  const read = (label) => {
    const value = raw.match(new RegExp(`^\\s*${label}:\\s*(-?\\d+)`, "m"))?.[1];
    if (value === undefined) throw new Error(`xwininfo omitted ${label}: ${raw.trim()}`);
    return Number(value);
  };
  return {
    X: read("Absolute upper-left X"),
    Y: read("Absolute upper-left Y"),
    WIDTH: read("Width"),
    HEIGHT: read("Height"),
  };
}

function windowIds(pattern = "^Tine( — .*)?$") {
  try {
    return xdo("search", "--onlyvisible", "--name", pattern)
      .split(/\s+/)
      .filter(Boolean)
      // Tauri/Openbox may expose a small same-title helper surface; the graph
      // window is the largest candidate and owns the title-bar close control.
      .sort((a, b) => {
        try {
          const ga = geometry(a);
          const gb = geometry(b);
          return gb.WIDTH * gb.HEIGHT - ga.WIDTH * ga.HEIGHT;
        } catch {
          return 0;
        }
      });
  } catch {
    return [];
  }
}

function frameExtents(id) {
  const raw = execFileSync("xprop", ["-id", id, "_NET_FRAME_EXTENTS", "_GTK_FRAME_EXTENTS"], { encoding: "utf8", env });
  const values = raw.match(/=\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+)/)?.slice(1).map(Number);
  if (!values) throw new Error(`window manager exposed malformed frame extents: ${raw.trim()}`);
  const [left, right, top, bottom] = values;
  return { left, right, top, bottom };
}

function windowManagerReady() {
  try {
    return /_NET_SUPPORTING_WM_CHECK.*window id/i.test(
      execFileSync("xprop", ["-root", "_NET_SUPPORTING_WM_CHECK"], { encoding: "utf8", env }),
    );
  } catch {
    return false;
  }
}

function captureRoot(name) {
  try {
    execFileSync("import", ["-window", "root", path.join(ARTIFACTS, name)], {
      env,
      stdio: "ignore",
      timeout: 5_000,
    });
  } catch {
    // The WebDriver image below remains the portable screenshot fallback.
  }
}

async function exactElement(selector, text) {
  const expected = text.normalize("NFC");
  // WebKitWebDriver's getText() can report an empty string for a visible,
  // non-interactive div. Inspect text only to locate the rendered row, then
  // perform the actual user action through WebDriver below.
  const index = await browser.execute((candidateSelector, candidateText) =>
    [...document.querySelectorAll(candidateSelector)].findIndex((element) =>
      (element.textContent ?? "").trim().normalize("NFC") === candidateText
    ), selector, expected);
  return index >= 0 ? (await browser.$$(selector))[index] : undefined;
}

async function elementContaining(selector, text) {
  const expected = text.normalize("NFC");
  for (const element of await browser.$$(selector)) {
    if ((await element.getText()).normalize("NFC").includes(expected)) return element;
  }
  return undefined;
}

async function buttonContaining(text) {
  for (const button of await browser.$$("button")) {
    if ((await button.getText()).includes(text)) return button;
  }
  return undefined;
}

async function clickExactText(selector, text, label = text) {
  const element = await waitFor(() => exactElement(selector, text), 12_000, `visible ${label} control was not found`);
  await element.click();
  return element;
}

async function clickContainingText(selector, text, label = text) {
  const element = await waitFor(() => elementContaining(selector, text), 12_000, `visible ${label} control was not found`);
  await element.click();
  return element;
}

async function assertVisible(text, label) {
  await browser.waitUntil(async () => (await browser.$("body").getText()).includes(text), {
    timeout: 15_000,
    timeoutMsg: `${label} was not visible: ${JSON.stringify(text)}`,
  });
}

async function assertTitle(name) {
  const title = await browser.$("h1.page-title");
  await title.waitForExist({ timeout: 15_000 });
  await browser.waitUntil(async () =>
    (await title.getText()).trim().normalize("NFC") === name.normalize("NFC"), {
    timeout: 15_000,
    timeoutMsg: `did not open ${JSON.stringify(name)}`,
  });
}

async function openPageFromInventory(name) {
  let row = await exactElement(".nav-page", name);
  if (!row) {
    await clickContainingText(".nav-section-header", "ALL PAGES", "ALL PAGES inventory");
    row = await waitFor(() => exactElement(".nav-page", name), 15_000, `inventory did not list ${JSON.stringify(name)}`);
  }
  await row.click();
  await assertTitle(name);
}

async function waitForFileText(file, text, label) {
  await waitForPersistedFileText(file, (body) => body.includes(text), `${label} did not durably save ${JSON.stringify(text)}`, {
    timeoutMs: 15_000,
  });
}

async function focusCurrentEditor() {
  let editor = await browser.$(".page-blocks textarea.block-editor, textarea.block-editor");
  if (await editor.isExisting()) return editor;
  let target = await browser.$(".page-blocks .ls-block .block-content-wrapper, .page-blocks .ls-block .block-content");
  if (await target.isExisting()) {
    await target.click();
  } else {
    target = await browser.$(".page-trailing-block-target");
    await target.waitForExist({ timeout: 10_000 });
    await target.click();
  }
  editor = await browser.$(".page-blocks textarea.block-editor, textarea.block-editor");
  await editor.waitForExist({ timeout: 10_000 });
  return editor;
}

async function raceExistingDraftWithManagedWinner(path, retained, winner) {
  const result = await browser.executeAsync((managedPath, retainedText, winnerText, done) => {
    const editor = document.querySelector(".page-blocks textarea.block-editor, textarea.block-editor");
    if (!(editor instanceof HTMLTextAreaElement)) {
      done({ error: "existing-page editor was not mounted" });
      return;
    }
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
    setter?.call(editor, retainedText);
    editor.dispatchEvent(new InputEvent("input", {
      bubbles: true,
      inputType: "insertText",
      data: retainedText,
    }));
    const invoke = globalThis.__TAURI_INTERNALS__.invoke.bind(globalThis.__TAURI_INTERNALS__);
    invoke("get_page_by_path", { path: managedPath }).then((page) => {
      if (!page?.rev || !page.blocks?.[0]) throw new Error("managed winner could not load exact page");
      page.blocks[0].raw = winnerText;
      return invoke("save_page", {
        page,
        baseRev: page.rev,
        force: false,
        conflictEpoch: null,
        managedConflictObservation: null,
      });
    }).then(
      (revision) => done({ revision }),
      (error) => done({ error: String(error) }),
    );
  }, path, retained, winner);
  if (result?.error || typeof result?.revision !== "string") {
    throw new Error(`existing managed race setup failed: ${JSON.stringify(result)}`);
  }
  return result.revision;
}

async function raceNewDraftWithManagedCreator(name, retained, winner) {
  const result = await browser.executeAsync((pageName, retainedText, winnerText, done) => {
    const editor = document.querySelector(".page-blocks textarea.block-editor, textarea.block-editor");
    if (!(editor instanceof HTMLTextAreaElement)) {
      done({ error: "new-page editor was not mounted" });
      return;
    }
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
    setter?.call(editor, retainedText);
    editor.dispatchEvent(new InputEvent("input", {
      bubbles: true,
      inputType: "insertText",
      data: retainedText,
    }));
    globalThis.__TAURI_INTERNALS__.invoke("save_page", {
      page: {
        name: pageName,
        kind: "page",
        title: pageName,
        pre_block: null,
        blocks: [{ id: "e2e-concurrent-new", raw: winnerText, collapsed: false, children: [], properties: [] }],
        rev: null,
        format: "md",
        read_only: false,
        path: "",
        guide: false,
      },
      baseRev: null,
      force: false,
      conflictEpoch: null,
      managedConflictObservation: null,
    }).then(
      (revision) => done({ revision }),
      (error) => done({ error: String(error) }),
    );
  }, name, retained, winner);
  if (result?.error || typeof result?.revision !== "string") {
    throw new Error(`new-page managed race setup failed: ${JSON.stringify(result)}`);
  }
  return result.revision;
}

async function assertRetainedConflictDraft(text, label) {
  await assertVisible("changed outside this editor", `${label} conflict banner`);
  await browser.waitUntil(async () => browser.execute((expected) => {
    const editor = document.querySelector("textarea.block-editor");
    const keep = document.querySelector(".conflict-btn.keep");
    return editor instanceof HTMLTextAreaElement
      && editor.value.includes(expected)
      && keep instanceof HTMLButtonElement
      && !keep.disabled;
  }, text), {
    timeout: 15_000,
    timeoutMsg: `${label} did not retain its draft behind an actionable conflict`,
  });
}

async function saveManagedWinnerAndClickVisibleKeepMine(path, winner) {
  const result = await browser.executeAsync((managedPath, winnerText, done) => {
    const invoke = globalThis.__TAURI_INTERNALS__.invoke.bind(globalThis.__TAURI_INTERNALS__);
    invoke("get_page_by_path", { path: managedPath }).then((page) => {
      if (!page?.rev || !page.blocks?.[0]) throw new Error("newer winner could not load exact page");
      page.blocks[0].raw = winnerText;
      return invoke("save_page", {
        page,
        baseRev: page.rev,
        force: false,
        conflictEpoch: null,
        managedConflictObservation: null,
      });
    }).then((revision) => {
      const keep = [...document.querySelectorAll("button")]
        .find((button) => button.textContent?.trim() === "Keep mine");
      if (!(keep instanceof HTMLButtonElement) || keep.disabled) {
        throw new Error("visible Keep mine was not actionable before the newer winner");
      }
      keep.click();
      return revision;
    }).then(
      (revision) => done({ revision }),
      (error) => done({ error: String(error) }),
    );
  }, path, winner);
  if (result?.error || typeof result?.revision !== "string") {
    throw new Error(`newer-winner click race failed: ${JSON.stringify(result)}`);
  }
  return result.revision;
}

async function clickKeepMine() {
  const keep = await waitFor(() => buttonContaining("Keep mine"), 15_000, "Keep mine action was not visible");
  await browser.waitUntil(async () => !(await keep.getAttribute("disabled")), {
    timeout: 15_000,
    timeoutMsg: "Keep mine did not regain revision-bound authority",
  });
  await keep.click();
}

async function editCurrentPage(marker, file, label) {
  const editor = await focusCurrentEditor();
  await editor.addValue(marker);
  // Leaving the ordinary editor is the real user save path; the Markdown file
  // is then the durable semantic oracle, not an implementation sidecar.
  await (await browser.$("h1.page-title")).click();
  await waitForFileText(file, marker, label);
}

async function openSyncSettings() {
  const trigger = await browser.$('button[title^="Settings"]');
  await trigger.waitForExist({ timeout: 10_000 });
  await trigger.click();
  await browser.$(".settings-modal").waitForExist({ timeout: 10_000 });
  await clickExactText(
    ".settings-nav-item",
    "Backups & recovery",
    "Backups & recovery settings tab",
  );
  await assertVisible("Storage & sync", "storage-and-sync settings");
  const experimental = await browser.$(".settings-experimental .settings-advanced-toggle");
  await experimental.waitForExist({ timeout: 10_000 });
  if ((await experimental.getAttribute("aria-expanded")) !== "true") {
    await experimental.click();
  }
  await assertVisible("Testing only.", "experimental managed-storage disclosure");
}

async function closeSettings() {
  for (let attempt = 0; attempt < 3 && await browser.$(".settings-modal").isExisting(); attempt += 1) {
    await browser.keys(["Escape"]);
    await sleep(100);
  }
  await browser.$(".settings-modal").waitForExist({ reverse: true, timeout: 10_000 });
}

async function acceptNativeConfirmation(label, before) {
  const dialogId = await waitFor(() => windowIds("^Tine$").find((id) => !before.has(id)), 12_000,
    `${label} did not open the real native confirmation dialog`);
  const title = xdo("getwindowname", dialogId);
  if (title !== "Tine") throw new Error(`${label} native dialog had unexpected title ${JSON.stringify(title)}`);
  try {
    execFileSync("import", ["-window", dialogId, path.join(ARTIFACTS, `native-confirm-${label}.png`)], {
      env,
      stdio: "ignore",
      timeout: 5_000,
    });
  } catch {}
  xdo("windowactivate", "--sync", dialogId);
  // rfd's GTK Yes/No action exposes the standard Yes mnemonic. This is native
  // keyboard input directed at the real dialog, never a DOM confirmation shim.
  xdo("key", "--clearmodifiers", "alt+y");
  await waitFor(() => !windowIds("^Tine$").includes(dialogId), 10_000,
    `${label} native confirmation did not accept Yes`);
  receipt.nativeConfirmations.push({ label, title });
}

async function clickButtonAndConfirm(text, label) {
  const button = await waitFor(() => buttonContaining(text), 12_000, `visible ${text} action was not found`);
  const before = new Set(windowIds("^Tine$"));
  await button.click();
  await acceptNativeConfirmation(label, before);
}

async function closeThroughTineControl(label) {
  const id = await waitFor(() => windowIds()[0], 12_000, `${label}: Tine window did not appear`);
  const pid = Number(xdo("getwindowpid", id));
  if (!Number.isInteger(pid) || pid <= 0) throw new Error(`${label}: xdotool returned invalid Tine PID ${JSON.stringify(pid)}`);
  const extents = frameExtents(id);
  if (extents.top < 1) throw new Error(`${label}: Tine did not expose its real native close control`);
  const bounds = geometry(id);
  const closeX = bounds.X + bounds.WIDTH - Math.max(10, Math.floor(extents.right / 2));
  const closeY = bounds.Y - Math.max(1, Math.floor(extents.top / 2));
  captureRoot(`native-close-${label}.png`);
  xdo("mousemove", "--sync", String(closeX), String(closeY));
  xdo("click", "1");
  await waitFor(() => windowIds().length === 0 && !processAlive(pid), 15_000,
    `${label}: real native close control did not terminate Tine; geometry=${JSON.stringify(bounds)} extents=${JSON.stringify(extents)} click=${closeX},${closeY}`);
  appPid = undefined;
  receipt.nativeCloses.push({ label, extents });
}

let browser;
let driver;
let driverLog;
let appPid;
let wm;
let wmLog;
let phase = "setup";
const receipt = {
  schemaVersion: 1,
  scenario: "sparse-v2-recovery",
  testedCommit: gitRevision(),
  app: APP,
  fixture: {
    normalPage: NORMAL_PAGE,
    nestedPagePath: "pages/研究/Résumé 日本語.md",
    journalPath: `journals/${journalStem}.md`,
    config: "logseq/config.edn",
  },
  nativeConfirmations: [],
  nativeCloses: [],
  milestones: {},
};

async function stopDriver() {
  const current = driver;
  driver = undefined;
  try { if (current?.pid) process.kill(-current.pid, "SIGKILL"); } catch {}
  if (current?.pid) {
    await waitFor(() => current.exitCode !== null || !processAlive(current.pid), 8_000,
      "tauri-driver process group did not stop during cleanup");
  }
  try { if (driverLog !== undefined) fs.closeSync(driverLog); } catch {}
  driverLog = undefined;
}

async function stopApp() {
  const pid = appPid;
  appPid = undefined;
  try { if (pid && processAlive(pid)) process.kill(pid, "SIGKILL"); } catch {}
  if (pid) {
    await waitFor(() => !processAlive(pid), 8_000, "Tine application process did not stop during cleanup");
  }
}

async function stopWindowManager() {
  const current = wm;
  wm = undefined;
  try { if (current?.pid) process.kill(-current.pid, "SIGKILL"); } catch {}
  if (current?.pid) {
    await waitFor(() => current.exitCode !== null || !processAlive(current.pid), 8_000,
      "Openbox process group did not stop during cleanup");
  }
  try { if (wmLog !== undefined) fs.closeSync(wmLog); } catch {}
  wmLog = undefined;
}

async function connect(label) {
  driverLog = fs.openSync(path.join(ARTIFACTS, `${label}-tauri-driver.log`), "w");
  driver = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", WD], {
    env,
    stdio: ["ignore", driverLog, driverLog],
    detached: true,
  });
  await sleep(2500);
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60_000,
    capabilities: {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: APP },
    },
  });
  await browser.$(".ls-block, .page-title, .journal-day").waitForExist({ timeout: 30_000 });
  const id = await waitFor(() => windowIds()[0], 12_000, `${label}: Tine native window did not appear`);
  const pid = Number(xdo("getwindowpid", id));
  if (!Number.isInteger(pid) || pid <= 0) throw new Error(`${label}: xdotool returned invalid Tine PID ${JSON.stringify(pid)}`);
  appPid = pid;
  receipt.milestones[label] = { startedExactApp: APP, profile: XDG };
}

function failureClassification(error) {
  const message = String(error);
  if (/HARNESS UNAVAILABLE|tauri-driver|WebKit|xdotool|Openbox|window manager|DISPLAY/i.test(message)) return "infrastructure";
  if (/native confirmation|native close control|did not open|did not durably save|was not visible|inventory did not list/i.test(message)) return "product";
  return "ambiguous";
}

try {
  phase = "start window manager";
  wmLog = fs.openSync(path.join(ARTIFACTS, "openbox.log"), "w");
  wm = spawn(process.env.E2E_WINDOW_MANAGER || "openbox", ["--sm-disable"], {
    env,
    stdio: ["ignore", wmLog, wmLog],
    detached: true,
  });
  await waitFor(() => wm.exitCode === null && windowManagerReady(), 10_000,
    "window manager did not become ready for the native sparse-v2 journey");

  phase = "legacy first launch";
  await connect("legacy-first-launch");
  await clickContainingText(".nav-item", "Journals", "Journals navigation");
  await assertVisible(JOURNAL_MARKER, "today's canonical journal");
  await openPageFromInventory(NESTED_PAGE);
  await assertVisible(NESTED_ORIGINAL, "legacy nested UTF page content");
  receipt.milestones.legacyFirstLaunch = {
    journalVisible: true,
    nestedPageVisible: true,
    nestedOriginalVisible: true,
  };

  phase = "activate sparse v2";
  await openSyncSettings();
  await clickButtonAndConfirm("Enable Tine-managed storage...", "enable-tine-managed-storage");
  await assertVisible("Tine-managed storage active", "active Tine-managed storage status");
  await closeSettings();

  phase = "sparse v2 existing-page conflict resolution";
  await openPageFromInventory(NESTED_PAGE);
  await assertVisible(NESTED_ORIGINAL, "nested UTF content after sparse-v2 activation without reload");
  await focusCurrentEditor();
  const retainedExisting = `${NESTED_ORIGINAL} ${NESTED_EDIT}`;
  await raceExistingDraftWithManagedWinner(
    "pages/研究/Résumé 日本語.md",
    retainedExisting,
    NESTED_WINNER_A,
  );
  await waitForFileText(NESTED_FILE, NESTED_WINNER_A, "existing-page winner A");
  await assertRetainedConflictDraft(NESTED_EDIT, "existing-page winner A");

  // Winner B lands after the banner observed A. Its actor save and the visible
  // Keep mine click are issued in one browser task, before a watcher refresh can
  // silently upgrade that click to B's authority. The first click must refuse,
  // preserve B, and re-arm the same retained draft against B.
  await saveManagedWinnerAndClickVisibleKeepMine(
    "pages/研究/Résumé 日本語.md",
    NESTED_WINNER_B,
  );
  await waitForFileText(NESTED_FILE, NESTED_WINNER_B, "newer existing-page winner B");
  await assertVisible("Couldn't overwrite", "newer-winner Keep mine refusal");
  await assertRetainedConflictDraft(NESTED_EDIT, "newer existing-page winner B");
  if (!fs.readFileSync(NESTED_FILE, "utf8").includes(NESTED_WINNER_B)) {
    throw new Error("the first Keep mine crossed a newer managed winner instead of conflicting again");
  }
  await clickKeepMine();
  await waitForFileText(NESTED_FILE, NESTED_EDIT, "revision-bound existing Keep mine");
  await browser.$(".conflict-banner").waitForExist({ reverse: true, timeout: 15_000 });
  receipt.milestones.sparseV2ExistingPage = {
    inventoryVisible: true,
    retainedDraft: NESTED_EDIT,
    firstWinner: NESTED_WINNER_A,
    newerWinnerRefused: NESTED_WINNER_B,
    saved: NESTED_EDIT,
  };

  phase = "sparse v2 concurrent new-page conflict resolution";
  const newPage = await browser.$("button.new-page-btn");
  await newPage.waitForExist({ timeout: 10_000 });
  await newPage.click();
  const switcher = await browser.$("input.switcher-input");
  await switcher.waitForExist({ timeout: 10_000 });
  await switcher.setValue(NEW_PAGE);
  const createRow = await waitFor(async () => {
    for (const row of await browser.$$(".switcher-row")) {
      if ((await row.getText()).includes(`Create page: ${NEW_PAGE}`)) return row;
    }
    return undefined;
  }, 12_000, `Create page result was not visible for ${JSON.stringify(NEW_PAGE)}`);
  await createRow.click();
  await assertTitle(NEW_PAGE);
  await focusCurrentEditor();
  await raceNewDraftWithManagedCreator(NEW_PAGE, NEW_EDIT, NEW_WINNER);
  await waitForFileText(NEW_FILE, NEW_WINNER, "concurrent new-page winner");
  await assertRetainedConflictDraft(NEW_EDIT, "concurrent new-page winner");
  await clickKeepMine();
  await waitForFileText(NEW_FILE, NEW_EDIT, "concurrent new-page Keep mine");
  await browser.$(".conflict-banner").waitForExist({ reverse: true, timeout: 15_000 });
  receipt.milestones.sparseV2NewPage = {
    createdThroughVisibleUi: true,
    retainedDraft: NEW_EDIT,
    concurrentWinner: NEW_WINNER,
    saved: NEW_EDIT,
  };

  phase = "restart sparse v2 authority";
  await closeThroughTineControl("sparse-v2-restart");
  try { await browser.deleteSession(); } catch {}
  browser = undefined;
  await stopDriver();
  await connect("sparse-v2-restart");
  await openPageFromInventory(NESTED_PAGE);
  await assertVisible(NESTED_EDIT, "saved existing page after sparse-v2 restart");
  await openPageFromInventory(NEW_PAGE);
  await assertVisible(NEW_EDIT, "saved new page after sparse-v2 restart");
  receipt.milestones.sparseV2Restart = { existingPageRendered: true, newPageRendered: true };

  phase = "return to standard Markdown mode";
  await openSyncSettings();
  await clickButtonAndConfirm("Return to Direct files", "return-to-direct-files");
  await assertVisible("Enable Tine-managed storage...", "Direct files status");
  await closeSettings();

  phase = "standard Markdown save and restart";
  await openPageFromInventory(NEW_PAGE);
  await editCurrentPage(STANDARD_EDIT, NEW_FILE, "new page after standard Markdown rollback");
  await closeThroughTineControl("standard-markdown-restart");
  try { await browser.deleteSession(); } catch {}
  browser = undefined;
  await stopDriver();
  await connect("standard-markdown-restart");
  await openPageFromInventory(NORMAL_PAGE);
  await assertVisible(NORMAL_MARKER, "ordinary page after standard Markdown restart");
  await openPageFromInventory(NESTED_PAGE);
  await assertVisible(NESTED_ORIGINAL, "nested original content after rollback restart");
  await assertVisible(NESTED_EDIT, "nested sparse-v2 edit after rollback restart");
  await openPageFromInventory(NEW_PAGE);
  await assertVisible(NEW_EDIT, "new sparse-v2 edit after rollback restart");
  await assertVisible(STANDARD_EDIT, "new standard Markdown edit after rollback restart");
  for (const [file, text, label] of [
    [NORMAL_FILE, NORMAL_MARKER, "ordinary Markdown page"],
    [NESTED_FILE, NESTED_EDIT, "nested UTF Markdown page"],
    [NEW_FILE, NEW_EDIT, "new Markdown page sparse-v2 edit"],
    [NEW_FILE, STANDARD_EDIT, "new Markdown page standard-mode edit"],
  ]) await waitForFileText(file, text, label);
  receipt.milestones.standardMarkdownRestart = {
    allRendered: true,
    markdownSemanticText: true,
  };

  phase = "final native close";
  await closeThroughTineControl("final-cleanup");
  try { await browser.deleteSession(); } catch {}
  browser = undefined;
  await stopDriver();
  receipt.result = "pass";
  fs.writeFileSync(path.join(ARTIFACTS, "sparse-v2-recovery-receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  console.log(`PASS: sparse-v2 activation, edit, native restart, rollback, and standard-Markdown continuity held: ${JSON.stringify(receipt.milestones)}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(ARTIFACTS, "failure.png")); } catch {}
  captureRoot("native-failure.png");
  const failure = {
    testedCommit: receipt.testedCommit,
    journey: "sparse-v2-recovery",
    phase,
    expected: "Activation and rollback preserve visible, durably saved page content across real native restarts.",
    observed: String(error).split("\n").slice(0, 4).join(" | "),
    classification: failureClassification(error),
    screenshot: "failure.png",
  };
  fs.writeFileSync(path.join(ARTIFACTS, "failure-capsule.json"), `${JSON.stringify(failure, null, 2)}\n`);
  console.error(`E2E FAILURE CAPSULE ${JSON.stringify(failure)}`);
  process.exitCode = 1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { await stopApp(); } catch {}
  try { await stopDriver(); } catch {}
  try { await stopWindowManager(); } catch {}
}

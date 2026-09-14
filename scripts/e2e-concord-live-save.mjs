// Harvest B3 live-conflict capsule matrix.
//
// For Direct Files and Tine-managed storage, two retained drafts race external
// atomic replacements. A full process restart must restore both exact drafts;
// one is resolved to the retained draft and one to the current storage owner.
// The graph-keyed native capsule is observed before restart, shrinks after the
// first resolution, and is durably absent before the second success is shown.
//
// Usage: TINE_APP=/path/to/tine node scripts/e2e-concord-live-save.mjs
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureDisplay } from "./lib/e2e-display.mjs";
import { openPageByLink } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import {
  freeLoopbackPort,
  tauriCapabilities,
  webdriverServerArgs,
} from "./e2e-capabilities.mjs";

await ensureDisplay();

const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD = process.env.TAURI_DRIVER
  || (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const DRIVER_PORT = process.env.E2E_DRIVER_PORT
  ? Number(process.env.E2E_DRIVER_PORT)
  : await freeLoopbackPort();
const NATIVE_PORT = process.env.E2E_NATIVE_PORT
  ? Number(process.env.E2E_NATIVE_PORT)
  : await freeLoopbackPort(new Set([DRIVER_PORT]));
const RUN_LABEL = (process.env.TINE_E2E_RUN_LABEL || "concord-live-save")
  .replaceAll(/[^A-Za-z0-9_.-]/g, "-");
const ARTIFACTS = path.resolve(process.env.E2E_ARTIFACT_DIR || "/tmp");
const MISSING_TARGET = process.env.TINE_E2E_MISSING_TARGET === "1";
fs.mkdirSync(ARTIFACTS, { recursive: true });

function waitFor(check, timeout, message, interval = 100) {
  const deadline = Date.now() + timeout;
  return (async () => {
    let last;
    while (Date.now() < deadline) {
      try {
        const value = await check();
        if (value) return value;
      } catch (error) {
        last = error;
      }
      await sleep(interval);
    }
    throw new Error(`${message}${last ? `; last observation: ${String(last)}` : ""}`);
  })();
}

function windowIds(env, pattern = "^Tine$") {
  try {
    return execFileSync("xdotool", ["search", "--onlyvisible", "--name", pattern], {
      encoding: "utf8",
      env,
    }).trim().split(/\s+/).filter(Boolean);
  } catch {
    return [];
  }
}

function processAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

function atomicReplace(file, text) {
  const replacement = `${file}.external`;
  fs.writeFileSync(replacement, text);
  fs.renameSync(replacement, file);
}

function capsuleFiles(root) {
  const found = [];
  const walk = (dir) => {
    let entries;
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
    for (const entry of entries) {
      const candidate = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(candidate);
      else if (entry.name.endsWith(".v1.json") && candidate.includes("conflict-capsules")) {
        found.push(candidate);
      }
    }
  };
  walk(root);
  return found;
}

function readCapsule(root) {
  const files = capsuleFiles(root);
  if (files.length !== 1) throw new Error(`expected one capsule file, found ${files.length}`);
  return { file: files[0], envelope: JSON.parse(fs.readFileSync(files[0], "utf8")) };
}

function assertCapsuleRecord(capsule, item, mode, expectedBaseRev) {
  if (!capsule
    || capsule.source !== "live-save"
    || capsule.page_name !== item.name
    || capsule.page_path !== item.path
    || capsule.kind !== "page"
    || !capsule.live) {
    throw new Error(`${mode}:${item.name}: graph/page capsule binding is invalid`);
  }
  if (!JSON.stringify(capsule.live.page).includes(item.local)) {
    throw new Error(`${mode}:${item.name}: exact retained draft bytes are missing`);
  }
  if (typeof capsule.live.base_rev !== "string"
    || (expectedBaseRev !== undefined && capsule.live.base_rev !== expectedBaseRev)) {
    throw new Error(`${mode}:${item.name}: exact load baseline is missing or changed`);
  }
  if (mode === "managed") {
    if (capsule.live.disk_rev !== undefined || capsule.live.conflict_epoch !== -1) {
      throw new Error(`${mode}:${item.name}: replacement authority leaked into the durable capsule`);
    }
  } else if (typeof capsule.live.disk_rev !== "string") {
    throw new Error(`${mode}:${item.name}: durable Direct disk revision is missing`);
  }
}

async function visibleButtonContaining(browser, text) {
  for (const button of await browser.$$("button")) {
    if (await button.isDisplayed() && (await button.getText()).includes(text)) return button;
  }
  return undefined;
}

async function acceptNativeConfirmation(env, label, before) {
  const dialog = await waitFor(
    () => windowIds(env).find((id) => !before.has(id)),
    30_000,
    `${label} did not show its native confirmation`,
  );
  execFileSync("xdotool", ["windowactivate", "--sync", dialog], { env });
  execFileSync("xdotool", ["key", "--clearmodifiers", "alt+y"], { env });
  await waitFor(() => !windowIds(env).includes(dialog), 30_000, `${label} confirmation did not close`);
}

async function enableManagedStorage(browser, env) {
  const trigger = await browser.$('button[title^="Settings"]');
  await trigger.waitForDisplayed({ timeout: 30_000 });
  await trigger.click();
  await browser.$(".settings-modal").waitForDisplayed({ timeout: 30_000 });
  const tab = await waitFor(
    () => visibleButtonContaining(browser, "Backups & recovery"),
    30_000,
    "Backups & recovery settings tab was absent",
  );
  await tab.click();
  const experimental = await browser.$(".settings-experimental .settings-advanced-toggle");
  await experimental.waitForDisplayed({ timeout: 30_000 });
  if ((await experimental.getAttribute("aria-expanded")) !== "true") await experimental.click();
  const action = await waitFor(
    () => visibleButtonContaining(browser, "Enable Tine-managed storage..."),
    30_000,
    "managed activation action was absent",
  );
  const before = new Set(windowIds(env));
  await action.click();
  await acceptNativeConfirmation(env, "managed activation", before);
  await browser.waitUntil(async () => (await browser.$("body").getText()).includes("Tine-managed storage active"), {
    timeout: 300_000,
    interval: 250,
    timeoutMsg: "managed activation did not reach active",
  });
  const close = await browser.$(".settings-pane-head .icon-btn:not(.settings-maximize)");
  await close.click();
  await browser.$(".settings-modal").waitForExist({ reverse: true, timeout: 30_000 });
}

async function openPage(browser, title) {
  // Route to the journal feed first: the page-ref links only exist there. The
  // `[[ ]]` tolerance this journey needed (`:ui/show-brackets?` defaults to on)
  // is now the shared link route's contract, along with re-finding the link on
  // each attempt. See scripts/lib/e2e-navigation.mjs.
  const journals = await browser.$(".nav-item*=Journals");
  await journals.waitForClickable({ timeout: 30_000 });
  await journals.click();
  await openPageByLink(browser, title, { timeout: 30_000 });
}

async function editPage(browser, text) {
  await browser.$(".page-blocks .ls-block .block-content").click();
  const editor = await browser.$(".page-blocks textarea.block-editor");
  await editor.waitForExist({ timeout: 5_000 });
  await browser.execute((next) => {
    const textarea = document.querySelector(".page-blocks textarea.block-editor");
    if (!(textarea instanceof HTMLTextAreaElement)) throw new Error("editor missing");
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
    setter?.call(textarea, next);
    textarea.dispatchEvent(new InputEvent("input", {
      bubbles: true,
      inputType: "insertText",
      data: next,
    }));
  }, text);
}

async function assertLiveConflict(browser, local, current, phase) {
  const resolver = await browser.$(".page-conflict");
  await resolver.waitForExist({ timeout: 30_000 });
  await browser.waitUntil(async () => {
    const state = await browser.execute(() => ({
      text: document.querySelector(".page-conflict")?.textContent ?? "",
      labels: [...document.querySelectorAll(".page-conflict .sync-merge-collabels span")]
        .map((label) => label.textContent?.trim() ?? ""),
    }));
    return state.text.includes("Your draft and the current file both changed")
      && state.text.includes(local)
      && state.text.includes(current)
      && state.labels.includes("Your retained draft")
      && state.labels.includes("Current file on disk");
  }, { timeout: 30_000, timeoutMsg: `${phase}: resolver did not retain both sides` });
}

async function resolveEverywhere(browser, side) {
  // The resolve-everywhere buttons carry TWO labels: `.conflict-wide` ("Keep
  // <side label>") and `.conflict-narrow` ("All mine" / "All theirs"). CSS shows
  // exactly one, so a visible-text probe is really an assertion about pane
  // width — which is a non-requirement of this journey, and which the
  // full-pane-width Concord default legitimately flipped. Identify the action
  // by the stable narrow label that is always present in the DOM, and click the
  // real button that owns it.
  const selector = side === "mine" ? "All mine" : "All theirs";
  const choose = await waitFor(async () => {
    for (const button of await browser.$$(".sync-merge-toolbar-actions button")) {
      const owns = await button.execute(
        (node, want) => (node.textContent ?? "").includes(want),
        selector,
      );
      if (owns && (await button.isDisplayed())) return button;
    }
    return undefined;
  }, 15_000, `${selector} action was absent`);
  await choose.click();
  const apply = await waitFor(
    () => visibleButtonContaining(browser, "Apply resolution"),
    15_000,
    "Apply resolution action was absent",
  );
  await apply.click();
  await browser.waitUntil(async () => (await browser.$$(".page-conflict")).length === 0, {
    timeout: 30_000,
    timeoutMsg: "resolved capsule stayed visible",
  });
}

async function waitForApp(browser, phase) {
  await browser.$("body").waitForExist({ timeout: 30_000 });
  try {
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
  } catch (error) {
    const state = await browser.execute(() => ({
      body: document.body?.innerText,
      html: document.body?.innerHTML.slice(0, 2000),
      location: location.href,
    }));
    throw new Error(`${phase}: app content did not appear: ${JSON.stringify(state)}; ${String(error)}`);
  }
}

async function runBackend(mode) {
  const suffix = MISSING_TARGET ? "direct-missing" : mode === "managed" ? "managed" : "direct";
  const graph = `/tmp/tgraph-concord-live-save-${suffix}`;
  const xdg = `/tmp/txdg-concord-live-save-${suffix}`;
  const data = `${xdg}/data`;
  const mineName = "B3EKeepDraft";
  const theirsName = "B3EUseCurrent";
  const mineFile = `${graph}/pages/${mineName}.md`;
  const theirsFile = `${graph}/pages/${theirsName}.md`;
  fs.rmSync(graph, { recursive: true, force: true });
  fs.rmSync(xdg, { recursive: true, force: true });
  if (process.env.TINE_E2E_SEED_GRAPH) {
    fs.cpSync(path.resolve(process.env.TINE_E2E_SEED_GRAPH), graph, { recursive: true });
  }
  for (const dir of [`${graph}/pages`, `${graph}/journals`, `${graph}/logseq`, data, `${xdg}/config`, `${xdg}/cache`]) {
    fs.mkdirSync(dir, { recursive: true });
  }
  fs.writeFileSync(mineFile, "- common mine base\n");
  fs.writeFileSync(theirsFile, "- common theirs base\n");
  const now = new Date();
  const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
  fs.writeFileSync(`${graph}/journals/${journal}.md`, `- [[${mineName}]]\n- [[${theirsName}]]\n`);

  const env = {
    ...process.env,
    TINE_GRAPH: graph,
    XDG_DATA_HOME: data,
    XDG_CONFIG_HOME: `${xdg}/config`,
    XDG_CACHE_HOME: `${xdg}/cache`,
    WEBKIT_DISABLE_DMABUF_RENDERER: "1",
    LIBGL_ALWAYS_SOFTWARE: "1",
    WEBKIT_DISABLE_COMPOSITING_MODE: "1",
    GDK_BACKEND: "x11",
  };
  let driver;
  let driverLog;
  let browser;

  const startDriver = async (phase) => {
    driverLog = fs.openSync(path.join(
      ARTIFACTS,
      `${RUN_LABEL}-${suffix}-${phase}-tauri-driver.log`,
    ), "w");
    driver = spawn(
      TD,
      webdriverServerArgs(DRIVER_PORT, NATIVE_PORT, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
      { env, stdio: ["ignore", driverLog, driverLog], detached: true },
    );
    await sleep(3000);
  };
  const stopDriver = async (deleteSession) => {
    if (deleteSession && browser) {
      try { await browser.deleteSession(); } catch {}
    }
    browser = undefined;
    const pid = driver?.pid;
    if (pid) {
      try { process.kill(-pid, "SIGKILL"); } catch {}
      await waitFor(
        () => driver.exitCode !== null || !processAlive(pid),
        30_000,
        `${suffix}: tauri-driver did not stop`,
      );
    }
    driver = undefined;
    try { if (driverLog !== undefined) fs.closeSync(driverLog); } catch {}
    driverLog = undefined;
  };
  const newSession = async () => {
    const browser = await remote({
      hostname: "127.0.0.1",
      port: DRIVER_PORT,
      path: "/",
      capabilities: tauriCapabilities(APP, `concord-live-save-${suffix}`),
      logLevel: "error",
      connectionRetryCount: 1,
      connectionRetryTimeout: 60_000,
    });
    // Keep a genuinely spacious desktop pane for Concord's review-width
    // contract; narrow-pane behavior has its own container-query coverage.
    await browser.setWindowSize(1400, 900);
    return browser;
  };
  const forceKillApp = async () => {
    const window = await waitFor(
      // Graph selection gives the main window a graph-qualified title. The
      // plain title matcher above deliberately remains for native dialogs.
      () => windowIds(env, "^Tine( — .*)?$")[0],
      30_000,
      `${suffix}: native app window was absent before SIGKILL`,
    );
    const pid = Number(execFileSync("xdotool", ["getwindowpid", window], {
      encoding: "utf8",
      env,
    }).trim());
    if (!Number.isInteger(pid) || pid <= 0) {
      throw new Error(`${suffix}: native window exposed invalid app pid ${pid}`);
    }
    const executable = fs.realpathSync(`/proc/${pid}/exe`);
    if (executable !== fs.realpathSync(APP)) {
      throw new Error(`${suffix}: refusing to kill unexpected executable ${executable}`);
    }
    fs.writeFileSync(path.join(ARTIFACTS, `${suffix}-killed-app.json`), JSON.stringify({
      window, pid, executable,
      title: execFileSync("xdotool", ["getwindowname", window], { encoding: "utf8", env }).trim(),
    }, null, 2));
    process.kill(pid, "SIGKILL");
    await waitFor(() => !processAlive(pid), 30_000, `${suffix}: SIGKILL did not stop Tine pid ${pid}`);
    browser = undefined;
    await stopDriver(false);
  };

  try {
    await startDriver("initial");
    browser = await newSession();
    await waitForApp(browser, `${suffix}:initial`);
    if (mode === "managed") await enableManagedStorage(browser, env);

    const cases = [
      {
        name: mineName,
        path: `pages/${mineName}.md`,
        file: mineFile,
        local: `${suffix} retained laptop draft`,
        current: `${suffix} current phone body`,
        outage: `${suffix} newer phone body during outage`,
      },
      {
        name: theirsName,
        path: `pages/${theirsName}.md`,
        file: theirsFile,
        local: `${suffix} second retained draft`,
        current: `${suffix} second current body`,
        outage: `${suffix} newer second body during outage`,
      },
    ];
    for (const item of cases) {
      await openPage(browser, item.name);
      await editPage(browser, item.local);
      atomicReplace(item.file, `- ${item.current}\n`);
      await assertLiveConflict(browser, item.local, item.current, `${suffix}:${item.name}:initial`);
    }

    const beforeRestart = await waitFor(() => {
      const files = capsuleFiles(data);
      if (files.length !== 1) return undefined;
      const observed = readCapsule(data);
      return observed.envelope.capsules?.length === 2 ? observed : undefined;
    }, 30_000, `${suffix}: native capsule did not contain both retained drafts`);
    const byName = new Map(beforeRestart.envelope.capsules.map((capsule) => [capsule.page_name, capsule]));
    const baseRevs = new Map();
    for (const item of cases) {
      const capsule = byName.get(item.name);
      assertCapsuleRecord(capsule, item, mode);
      baseRevs.set(item.name, capsule.live.base_rev);
    }

    if (MISSING_TARGET) {
      const before = new Set(windowIds(env, "^Tine( — .*)?$"));
      const appWindow = [...before][0];
      if (!appWindow) throw new Error("close recovery: native app window absent");
      execFileSync("xdotool", ["windowactivate", "--sync", appWindow], { env });
      execFileSync("xdotool", ["key", "--clearmodifiers", "alt+F4"], { env });
      const dialog = await waitFor(() => windowIds(env, "^(Tine|Unsaved changes)$").find((id) => !before.has(id)),
        45_000, "failed close did not ask before discarding drafts");
      execFileSync("xdotool", ["windowactivate", "--sync", dialog], { env });
      execFileSync("xdotool", ["key", "--clearmodifiers", "alt+n"], { env });
      await browser.$(".unsaved-recovery-panel").waitForDisplayed({ timeout: 15_000 });
      const recovery = await browser.$(".unsaved-recovery-panel").getText();
      for (const item of cases) {
        if (!recovery.includes(item.name) || !recovery.includes(item.local)) {
          throw new Error(`close recovery omitted ${item.name} or its writing`);
        }
      }
      await browser.saveScreenshot(path.join(ARTIFACTS, "refused-close-recovery.png"));
      await (await visibleButtonContaining(browser, "Keep working")).click();
    }
    await forceKillApp();
    for (const item of cases) atomicReplace(item.file, `- ${item.outage}\n`);
    if (MISSING_TARGET) fs.unlinkSync(mineFile);
    await startDriver("restart");
    browser = await newSession();
    await waitForApp(browser, `${suffix}:restart`);

    const afterRestart = await waitFor(() => {
      const observed = readCapsule(data);
      return observed.envelope.capsules?.length === 2 ? observed : undefined;
    }, 30_000, `${suffix}: restart did not retain both durable capsules`);
    if (afterRestart.file !== beforeRestart.file) {
      throw new Error(`${suffix}: restart restored the capsules under a different graph binding`);
    }
    const restoredByName = new Map(
      afterRestart.envelope.capsules.map((capsule) => [capsule.page_name, capsule]),
    );
    for (const item of cases) {
      assertCapsuleRecord(restoredByName.get(item.name), item, mode, baseRevs.get(item.name));
    }

    if (MISSING_TARGET) {
      // Use the reported conflict entry, which pins the physical path. A plain
      // page link may legitimately open a prospective page by name instead.
      for (let attempt = 0; attempt < 2; attempt++) {
        await browser.$(".conflict-queue-badge").click();
        await browser.waitUntil(async () => (await browser.$$(".page-conflict")).length > 0,
          { timeout: 15_000, timeoutMsg: "conflict entry did not open its review" });
        if ((await browser.$$(".recovery-draft")).length) break;
      }
      await browser.$(".recovery-draft").waitForExist({ timeout: 15_000 });
      const preview = await browser.$(".recovery-draft pre").getText();
      if (!preview.includes(cases[0].local)) throw new Error("missing-target view lost retained writing");
      if (fs.existsSync(mineFile)) throw new Error("viewing missing-target recovery created a graph file");
      await browser.saveScreenshot(path.join(ARTIFACTS, "missing-target-recovery.png"));
    } else {
      await openPage(browser, mineName);
      await assertLiveConflict(browser, cases[0].local, cases[0].outage, `${suffix}:mine:restart`);
    }
    await resolveEverywhere(browser, "mine");
    await waitForFileText(mineFile, (text) => text.includes(cases[0].local), `${suffix}: keep retained draft`);
    const afterFirst = readCapsule(data).envelope.capsules;
    if (afterFirst.length !== 1 || afterFirst[0].page_name !== theirsName) {
      throw new Error(`${suffix}: first resolution was acknowledged before durable capsule retirement`);
    }

    await openPage(browser, theirsName);
    await assertLiveConflict(browser, cases[1].local, cases[1].outage, `${suffix}:theirs:restart`);
    await resolveEverywhere(browser, "theirs");
    await waitForFileText(theirsFile, (text) => text.includes(cases[1].outage), `${suffix}: use current owner`);
    if (capsuleFiles(data).length !== 0) {
      throw new Error(`${suffix}: final resolution was acknowledged before capsule file retirement`);
    }
    const legacy = await browser.execute(() => localStorage.getItem("tine.concord.live-conflicts.v1"));
    if (legacy !== null) throw new Error(`${suffix}: retired localStorage channel survived first use`);
    console.log(`PASS: ${suffix} SIGKILL restart restored exact capsules, re-observed newer owners, and resolved both sides`);
  } catch (error) {
    try {
      await browser.saveScreenshot(path.join(ARTIFACTS, `${suffix}-failure.png`));
      const state = await browser.execute(() => ({
        body: document.body.innerText,
        active: document.activeElement?.outerHTML,
        conflicts: [...document.querySelectorAll(".page-conflict")].map(el => el.outerHTML),
      }));
      fs.writeFileSync(path.join(ARTIFACTS, `${suffix}-failure.json`), JSON.stringify(state, null, 2));
    } catch {}
    throw error;
  } finally {
    try { await stopDriver(true); } catch {}
    await sleep(1500);
  }
}

// Xvfb supplies a display, but native dialog activation also needs an EWMH
// window manager. Keep that owner alive across both app SIGKILL/restart cases.
function windowManagerReady() {
  try {
    return /window id # 0x[1-9a-f][0-9a-f]*/i.test(execFileSync(
      "xprop", ["-root", "_NET_SUPPORTING_WM_CHECK"], { encoding: "utf8" },
    ));
  } catch { return false; }
}

let windowManager;
let windowManagerLog;
let failure;
try {
  if (!windowManagerReady()) {
    windowManagerLog = fs.openSync(path.join(ARTIFACTS, "openbox.log"), "w");
    windowManager = spawn(process.env.E2E_WINDOW_MANAGER || "openbox", ["--sm-disable"], {
      stdio: ["ignore", windowManagerLog, windowManagerLog],
    });
    let startError;
    windowManager.on("error", (error) => { startError = error; });
    await waitFor(() => {
      if (startError) throw startError;
      return windowManager.exitCode === null && windowManagerReady();
    }, 15_000, "window manager did not become ready");
  }
  fs.writeFileSync(path.join(ARTIFACTS, "window-manager.json"), JSON.stringify({
    ownedPid: windowManager?.pid ?? null,
    supportingWindow: execFileSync("xprop", ["-root", "_NET_SUPPORTING_WM_CHECK"], { encoding: "utf8" }).trim(),
  }, null, 2));
  await runBackend("direct");
  if (!MISSING_TARGET) await runBackend("managed");
  console.log("PASS: Harvest B3 Direct/Managed restart capsule matrix");
} catch (error) {
  failure = error;
  console.error("FAIL:", error?.stack ?? error);
} finally {
  windowManager?.kill("SIGTERM");
  if (windowManagerLog !== undefined) fs.closeSync(windowManagerLog);
}

process.exit(failure ? 1 : 0);

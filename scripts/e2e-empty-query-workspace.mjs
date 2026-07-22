// Linux real-WebKit regression for GH #172. This script intentionally stays at
// the visible Ctrl+K/pane/tab/modal boundary: no router calls or synthetic DOM
// events stand in for a user operation.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4492);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4493);
const TMP = process.env.E2E_TMP_DIR || "/tmp/tine-empty-query-workspace-e2e";
const GRAPH = path.join(TMP, "graph");
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || path.join(TMP, "artifacts");

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq", "config.edn"), "{}\n");
fs.writeFileSync(path.join(GRAPH, "pages", "Start.md"), "- Empty-workspace fixture result\n");

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg", "data"),
  XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"),
  XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};

function graphDigest() {
  const entries = [];
  const walk = (dir) => {
    for (const name of fs.readdirSync(dir).sort()) {
      const file = path.join(dir, name);
      const stat = fs.statSync(file);
      if (stat.isDirectory()) walk(file);
      else entries.push([path.relative(GRAPH, file), crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex")]);
    }
  };
  walk(GRAPH);
  return entries;
}

function sameDigest(before, label) {
  const after = graphDigest();
  if (JSON.stringify(before) !== JSON.stringify(after)) throw new Error(`${label} mutated the disposable graph: ${JSON.stringify({ before, after })}`);
}

async function withApp(index, fn) {
  const driverPort = DRIVER_BASE + index * 2;
  const nativePort = NATIVE_BASE + index * 2;
  const log = fs.openSync(path.join(TMP, `tauri-driver-${index}.log`), "w");
  const driver = spawn(TD, ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
    env, stdio: ["ignore", log, log], detached: true,
  });
  let browser;
  try {
    await sleep(2_500);
    browser = await remote({
      hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error",
      connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    });
    // A restored session may deliberately resume directly in a virtual query
    // workspace. The first-launch scenario below separately waits for .ls-block
    // before recording its graph-ready no-write baseline.
    await browser.$(".ls-block, .page-title, .query-workspace").waitForExist({ timeout: 20_000 });
    await fn(browser);
    // Give the graph-scoped session save its normal debounce before the orderly
    // WebDriver session deletion used for the restart proof.
    await sleep(900);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-driver.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

const paneSelector = (paneId) => `[data-pane-id="${paneId}"]`;

async function paneState(browser, paneId) {
  return browser.execute((id) => {
    const pane = document.querySelector(`[data-pane-id="${id}"]`);
    const workspace = pane?.querySelector(".query-workspace");
    const activeTab = pane?.querySelector(".tab.active")?.getAttribute("data-tab-id") ?? null;
    const source = workspace?.querySelector(".query-workspace-source");
    return {
      pane: id,
      routeId: workspace?.getAttribute("data-query-route-id") ?? null,
      source: source?.value ?? null,
      activeTab,
      sourceFocused: document.activeElement === source,
      status: workspace?.querySelector(".query-workspace-status")?.textContent?.trim() ?? null,
      presentation: workspace?.querySelector(".query-presentations button.active")?.textContent?.trim() ?? null,
    };
  }, paneId);
}

async function focusedPane(browser) {
  return browser.execute(() => {
    const focused = [...document.querySelectorAll(".pane-focused[data-pane-id]")];
    if (focused.length !== 1) {
      throw new Error(`expected exactly one logical focused pane, found ${focused.length}: ${JSON.stringify(focused.map((pane) => pane.getAttribute("data-pane-id")))}`);
    }
    return focused[0].getAttribute("data-pane-id");
  });
}

async function splitAndFocusOther(browser) {
  await browser.keys(["Control", "Alt", "\\"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll("[data-pane-id]").length === 2), {
    timeout: 8_000, timeoutMsg: "native split did not create a second pane",
  });
  const panes = await browser.execute(() => [...document.querySelectorAll("[data-pane-id]")].map((pane) => pane.getAttribute("data-pane-id")));
  const other = panes.find((id) => id && id !== "main");
  if (!other) throw new Error(`could not identify non-main pane: ${JSON.stringify(panes)}`);
  await browser.$(paneSelector(other)).click();
  return other;
}

async function openSwitcher(browser, paneId, source = "") {
  await browser.$(paneSelector(paneId)).click();
  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 5_000 });
  if (source) await input.setValue(source);
  return input;
}

async function assertOpenButton(browser) {
  const button = await browser.$("[data-open-search-tab]");
  await button.waitForExist({ timeout: 5_000 });
  const contract = { enabled: await button.isEnabled(), text: (await button.getText()).trim() };
  if (!contract.enabled || contract.text !== "Open search tab") throw new Error(`GH #172 Ctrl+K footer contract failed: ${JSON.stringify(contract)}`);
  return button;
}

async function pointerOpen(browser) {
  const button = await assertOpenButton(browser);
  // WebdriverIO's move+click is a real WebDriver pointer action, unlike DOM
  // click()/dispatchEvent proxies. Keep both calls explicit for the overlay
  // capture-phase pane-retarget regression.
  await button.moveTo();
  await button.click();
}

async function keyboardOpen(browser) {
  // Put real native focus on the switcher input using the browser document;
  // WebKitWebDriver has no element.focus() command.
  await browser.execute(() => document.querySelector(".switcher-input")?.focus());
  for (let attempts = 0; attempts < 24; attempts += 1) {
    if (await browser.execute(() => document.activeElement?.hasAttribute("data-open-search-tab"))) break;
    await browser.keys("Tab");
  }
  if (!await browser.execute(() => document.activeElement?.hasAttribute("data-open-search-tab"))) {
    throw new Error("native Tab traversal did not reach Open search tab");
  }
  await browser.keys("Enter");
}

async function clickTab(browser, paneId, tabId) {
  const tab = await browser.$(`${paneSelector(paneId)} .tab[data-tab-id="${tabId}"]`);
  await tab.waitForExist({ timeout: 5_000 });
  await tab.click();
}

async function sourceFocusProbeStart(browser, paneId) {
  await browser.execute((id) => {
    const key = "__gh172SourceFocusProbe";
    const previous = window[key];
    if (previous) document.removeEventListener("focusin", previous.listener, true);
    const probe = { count: 0, paneId: id, listener: null };
    probe.listener = (event) => {
      const pane = document.querySelector(`[data-pane-id="${probe.paneId}"]`);
      if (event.target === pane?.querySelector(".query-workspace-source")) probe.count += 1;
    };
    document.addEventListener("focusin", probe.listener, true);
    window[key] = probe;
  }, paneId);
}

async function sourceFocusProbeStop(browser) {
  return browser.execute(() => {
    const key = "__gh172SourceFocusProbe";
    const probe = window[key];
    if (!probe) throw new Error("missing GH #172 source-focus probe");
    document.removeEventListener("focusin", probe.listener, true);
    delete window[key];
    return probe.count;
  });
}

async function activateRestoredQueryTab(browser, paneId, expectedRouteId) {
  // Snapshot real, pane-scoped restored controls. data-tab-id is generated per
  // process and only addresses these controls; mounted routeId is the persisted
  // identity under test.
  const inactiveTabIds = await browser.execute((id) => {
    const pane = document.querySelector(`[data-pane-id="${id}"]`);
    return [...(pane?.querySelectorAll(".tab:not(.active)") ?? [])]
      .map((tab) => tab.getAttribute("data-tab-id"))
      .filter((tabId) => tabId !== null);
  }, paneId);
  const attempted = [];
  for (const tabId of inactiveTabIds) {
    const tab = await browser.$(`${paneSelector(paneId)} .tab[data-tab-id="${tabId}"]`);
    await tab.waitForExist({ timeout: 5_000 });
    await tab.moveTo();
    await tab.click();
    await browser.waitUntil(async () => (await paneState(browser, paneId)).activeTab === tabId, {
      timeout: 8_000, timeoutMsg: `restored tab control ${tabId} did not activate in ${paneId}`,
    });
    const state = await paneState(browser, paneId);
    attempted.push({ tabId, routeId: state.routeId });
    if (state.routeId === expectedRouteId) return state;
  }
  throw new Error(`no restored inactive tab in ${paneId} reached expected route ${expectedRouteId}: ${JSON.stringify({ inactiveTabIds, attempted })}`);
}

async function setSource(browser, paneId, source) {
  const input = await browser.$(`${paneSelector(paneId)} .query-workspace-source`);
  await input.waitForExist({ timeout: 5_000 });
  await input.setValue(source);
}

let origin;
let firstEmpty;
let secondSearch;
let mainEmpty;
let beforeRestart;
let postInitializationGraph;

await withApp(0, async (browser) => {
  // (1) App/graph readiness precedes the full-recursive no-write baseline.
  await browser.$(".ls-block").waitForExist({ timeout: 15_000 });
  const graphAfterInitialization = graphDigest();
  postInitializationGraph = graphAfterInitialization;
  origin = await splitAndFocusOther(browser);

  // (2) Non-main pointer promotion, pane-scoped identity/focus, neutral copy.
  await openSwitcher(browser, origin);
  await pointerOpen(browser);
  await browser.$(`${paneSelector(origin)} .query-workspace`).waitForExist({ timeout: 8_000 });
  firstEmpty = await paneState(browser, origin);
  if (firstEmpty.source !== "" || !firstEmpty.routeId || !firstEmpty.sourceFocused || firstEmpty.status !== "Enter a search to begin.") {
    throw new Error(`pointer promotion lost origin/focus/neutral empty state: ${JSON.stringify(firstEmpty)}`);
  }
  await browser.saveScreenshot(path.join(ARTIFACTS, "split-pointer-empty.png"));

  // (3) Empty naming rejection cannot write any graph child.
  const emptyTitle = await browser.$(`${paneSelector(origin)} .query-workspace-save input`);
  await emptyTitle.setValue("Empty query must remain virtual");
  await browser.$(`${paneSelector(origin)} .query-workspace-save button[type="submit"]`).click();
  await browser.$(`${paneSelector(origin)} .query-workspace-save-error`).waitForExist({ timeout: 5_000 });
  sameDigest(graphAfterInitialization, "empty query Save");

  // (4) Keyboard promotion creates a second identity, and real tab controls
  // prove that only the selected tab receives the distinct source update.
  await openSwitcher(browser, origin);
  await keyboardOpen(browser);
  await browser.waitUntil(async () => (await paneState(browser, origin)).routeId !== firstEmpty.routeId, {
    timeout: 8_000, timeoutMsg: "keyboard activation did not transition the origin route ID",
  });
  secondSearch = await paneState(browser, origin);
  if (!secondSearch.routeId || secondSearch.routeId === firstEmpty.routeId || !secondSearch.sourceFocused) {
    throw new Error(`keyboard promotion did not create a focused independent route: ${JSON.stringify(secondSearch)}`);
  }
  await setSource(browser, origin, "fixture");
  await browser.waitUntil(async () => (await paneState(browser, origin)).source === "fixture", { timeout: 8_000, timeoutMsg: "distinct source did not reach active second workspace" });
  secondSearch = await paneState(browser, origin);
  await clickTab(browser, origin, firstEmpty.activeTab);
  await browser.waitUntil(async () => (await paneState(browser, origin)).routeId === firstEmpty.routeId, { timeout: 8_000, timeoutMsg: "first empty tab did not reactivate" });
  const firstAgain = await paneState(browser, origin);
  if (firstAgain.source !== "" || firstAgain.routeId === secondSearch.routeId) throw new Error(`query tabs lost independent state: ${JSON.stringify({ firstAgain, secondSearch })}`);

  // (5) Put independent active query routes in both panes and deliberately
  // choose the persisted focus owner. The original empty origin tab remains
  // the inactive sibling while the distinct-source route is active.
  await openSwitcher(browser, "main");
  await pointerOpen(browser);
  await browser.$(`${paneSelector("main")} .query-workspace`).waitForExist({ timeout: 8_000 });
  mainEmpty = await paneState(browser, "main");
  if (!mainEmpty.routeId || mainEmpty.source !== "") throw new Error(`main empty workspace was not created: ${JSON.stringify(mainEmpty)}`);
  await clickTab(browser, origin, secondSearch.activeTab);
  const activeOrigin = await paneState(browser, origin);
  const activeMain = await paneState(browser, "main");
  beforeRestart = {
    origin: { routeId: activeOrigin.routeId, source: activeOrigin.source, activeTab: activeOrigin.activeTab },
    main: { routeId: activeMain.routeId, source: activeMain.source, activeTab: activeMain.activeTab },
    inactiveOriginRouteId: firstEmpty.routeId,
    focusedPane: await focusedPane(browser),
  };
  if (beforeRestart.focusedPane !== origin
      || activeOrigin.routeId !== secondSearch.routeId || activeOrigin.source !== "fixture" || activeOrigin.activeTab !== secondSearch.activeTab
      || activeMain.routeId !== mainEmpty.routeId || activeMain.source !== "" || activeMain.activeTab !== mainEmpty.activeTab
      || activeOrigin.activeTab === firstEmpty.activeTab || !activeOrigin.activeTab || !activeMain.activeTab
      || !activeOrigin.sourceFocused || activeMain.sourceFocused) {
    throw new Error(`pre-restart split ownership is wrong: ${JSON.stringify(beforeRestart)}`);
  }
  sameDigest(graphAfterInitialization, "virtual query workspace session work");
});

await withApp(1, async (browser) => {
  // (6) Exact split active route/source and focused-pane restoration. The
  // mounted workspace route is authoritative here; data-tab-id is regenerated
  // when the process restores tab controls.
  const restoredOrigin = await paneState(browser, origin);
  const restoredMain = await paneState(browser, "main");
  const restoredFocus = await focusedPane(browser);
  if (restoredOrigin.routeId !== beforeRestart.origin.routeId
      || restoredOrigin.source !== beforeRestart.origin.source
      || restoredMain.routeId !== beforeRestart.main.routeId
      || restoredMain.source !== beforeRestart.main.source
      || !restoredOrigin.activeTab || !restoredMain.activeTab
      || restoredFocus !== origin || !restoredOrigin.sourceFocused || restoredMain.sourceFocused) {
    throw new Error(`restart failed to preserve pane active route/source/focus ownership: ${JSON.stringify({ restoredOrigin, restoredMain, restoredFocus, beforeRestart })}`);
  }
  await browser.saveScreenshot(path.join(ARTIFACTS, "restored-focus.png"));
  await sourceFocusProbeStart(browser, origin);
  const sibling = await activateRestoredQueryTab(browser, origin, beforeRestart.inactiveOriginRouteId);
  const siblingFocusCount = await sourceFocusProbeStop(browser);
  if (sibling.routeId !== firstEmpty.routeId || sibling.source !== "" || !sibling.sourceFocused || siblingFocusCount !== 1) {
    throw new Error(`sibling route/source/focus did not survive restart: ${JSON.stringify({ sibling, siblingFocusCount })}`);
  }
  const friendlyRestored = await activateRestoredQueryTab(browser, origin, secondSearch.routeId);
  if (friendlyRestored.routeId !== secondSearch.routeId || friendlyRestored.source !== "fixture" || !friendlyRestored.sourceFocused) {
    throw new Error(`friendly query route did not reactivate through its restored tab: ${JSON.stringify(friendlyRestored)}`);
  }

  // (7) Filters Apply and a real presentation button update one route without
  // handing focus back to its source merely because the route object changed.
  const routeBeforeFilters = friendlyRestored.routeId;
  const filters = await browser.$(`${paneSelector(origin)} .query-advanced-toggle`);
  await filters.click();
  const dialog = await browser.$(".query-advanced-modal");
  await dialog.waitForExist({ timeout: 5_000 });
  const friendlyInputs = await browser.$$(".query-advanced-modal .query-friendly-fields input");
  if (!friendlyInputs.length) throw new Error("Filters / Advanced did not expose friendly fields for the selected friendly query");
  await friendlyInputs[0].setValue("result");
  const apply = await browser.$(".query-advanced-modal .query-advanced-actions .primary");
  await apply.click();
  await dialog.waitForExist({ reverse: true, timeout: 5_000 });
  await browser.waitUntil(async () => (await paneState(browser, origin)).source.includes("result"), { timeout: 8_000, timeoutMsg: "Filters Apply did not update the selected route" });
  const afterApply = await paneState(browser, origin);
  if (afterApply.routeId !== routeBeforeFilters || await browser.execute((id) => document.activeElement === document.querySelector(`[data-pane-id="${id}"] .query-advanced-toggle`), origin) !== true) {
    throw new Error(`Filters Apply changed identity or lost its close/focus-return contract: ${JSON.stringify(afterApply)}`);
  }
  const presentationButtons = await browser.$$(`${paneSelector(origin)} .query-presentations button`);
  const tableButton = await (async () => {
    for (const candidate of presentationButtons) if ((await candidate.getText()).trim() === "Table") return candidate;
    return null;
  })();
  if (!tableButton) throw new Error("missing real Table presentation button");
  await tableButton.click();
  await browser.waitUntil(async () => (await paneState(browser, origin)).presentation === "Table", { timeout: 8_000, timeoutMsg: "presentation button did not update selected route" });
  const afterPresentation = await paneState(browser, origin);
  if (afterPresentation.routeId !== routeBeforeFilters || await browser.execute(() => document.activeElement?.textContent?.trim() === "Table") !== true) {
    throw new Error(`same-ID presentation update stole source focus: ${JSON.stringify(afterPresentation)}`);
  }

  // (8) Rust-invalid-but-JavaScript-valid friendly source remains promotable.
  await openSwitcher(browser, "main", "  /(a)\\1/  ");
  await browser.$(".switcher-error").waitForExist({ timeout: 10_000 });
  const invalidButton = await assertOpenButton(browser);
  await invalidButton.moveTo(); await invalidButton.click();
  await browser.waitUntil(async () => (await paneState(browser, "main")).source === "/(a)\\1/", { timeout: 8_000, timeoutMsg: "trimmed invalid source was not promoted" });
  const invalid = await paneState(browser, "main");
  const invalidDiagnostic = await browser.$(`${paneSelector("main")} .query-workspace-diagnostics [data-code="invalid_regex"]`);
  await invalidDiagnostic.waitForExist({ timeout: 10_000 });
  if (!invalid.routeId || invalid.source !== "/(a)\\1/") throw new Error(`invalid source route is wrong: ${JSON.stringify(invalid)}`);
  await browser.saveScreenshot(path.join(ARTIFACTS, "invalid-regex-diagnostic.png"));

  // (9) Invalid naming rejection is visible and graph-complete no-write proof.
  const beforeInvalidSave = graphDigest();
  await (await browser.$(`${paneSelector("main")} .query-workspace-save input`)).setValue("Invalid search must remain virtual");
  await (await browser.$(`${paneSelector("main")} .query-workspace-save button[type="submit"]`)).click();
  await browser.$(`${paneSelector("main")} .query-workspace-save-error`).waitForExist({ timeout: 5_000 });
  sameDigest(beforeInvalidSave, "Rust-invalid query Save");

  // (10) A valid workspace still executes under the existing bounded result path.
  await setSource(browser, origin, "fixture");
  await browser.waitUntil(async () => {
    const state = await paneState(browser, origin);
    return state.status !== "Enter a search to begin." && /result/.test(state.status ?? "");
  }, { timeout: 10_000, timeoutMsg: "valid source did not reach the bounded live query path" });
  sameDigest(postInitializationGraph, "valid virtual query execution");
});

console.log("PASS: GH #172 literal Linux WebKit split pointer/keyboard/restart/focus/materialization contract");

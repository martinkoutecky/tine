// Linux real-app proof for GH #101. Verifies independent Favorites/Recent
// disclosure, per-graph isolation across in-app graph switches, and restart
// persistence through Tine's native session files.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_BASE = Number(process.env.E2E_DRIVER_PORT || 4482);
const NATIVE_BASE = Number(process.env.E2E_NATIVE_PORT || 4483);
const TMP = "/tmp/tine-sidebar-sections-e2e";
const GRAPH_A = `${TMP}/graph-a`;
const GRAPH_B = `${TMP}/graph-b`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const [graph, suffix] of [[GRAPH_A, "A"], [GRAPH_B, "B"]]) {
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${graph}/${dir}`, { recursive: true });
  fs.writeFileSync(`${graph}/logseq/config.edn`, `{:favorites ["Favorite ${suffix}"]}\n`);
  fs.writeFileSync(`${graph}/pages/Favorite ${suffix}.md`, `- favorite ${suffix}\n`);
  fs.writeFileSync(`${graph}/pages/Recent ${suffix}.md`, `- recent ${suffix}\n`);
  const now = new Date();
  const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
  fs.writeFileSync(`${graph}/journals/${journal}.md`, `- open [[Recent ${suffix}]]\n`);
}
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
// Seed the disposable native known-graphs list without opening graph B first.
// Launching B merely to register it would leave Tine's single-instance graph
// registry free to focus that peer window instead of exercising an in-place
// switch in the A window under test.
const appData = `${TMP}/xdg/data/page.tine.Tine`;
fs.mkdirSync(appData, { recursive: true });
fs.writeFileSync(`${appData}/tine-settings.json`, JSON.stringify({
  known_graphs: [
    { name: "graph-a", path: GRAPH_A },
    { name: "graph-b", path: GRAPH_B },
  ],
  last_graph_path: GRAPH_A,
}, null, 2));

const baseEnv = {
  ...process.env,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};

async function withApp(graph, index, fn) {
  const driverPort = DRIVER_BASE + index * 2;
  const nativePort = NATIVE_BASE + index * 2;
  const logPath = `${TMP}/tauri-driver-${index}.log`;
  const log = fs.openSync(logPath, "w");
  const td = spawn(TD, ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
    env: { ...baseEnv, TINE_GRAPH: graph }, stdio: ["ignore", log, log], detached: true,
  });
  await sleep(2500);
  let browser;
  try {
    browser = await remote({
      hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    });
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
    await fn(browser);
    await sleep(500);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

async function navigatePage(browser, name) {
  for (const selector of [`a.page-ref=${name}`, `span.page-ref=${name}`, `*=${name}`]) {
    const link = await browser.$(selector);
    if (await link.isExisting()) { await link.click(); break; }
  }
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === name, {
    timeout: 10_000, timeoutMsg: `${name} did not open`,
  });
}

async function section(browser, name) {
  const control = await browser.$(`[data-sidebar-section="${name}"]`);
  await control.waitForExist({ timeout: 10_000 });
  return control;
}

async function expectExpanded(browser, name, expected) {
  const control = await section(browser, name);
  const actual = await control.getAttribute("aria-expanded");
  if (actual !== String(expected)) throw new Error(`${name} aria-expanded=${actual}, expected ${expected}`);
  return control;
}

async function activateSectionWithKeyboard(browser, name) {
  await browser.execute((sectionName) => {
    document.querySelector(`[data-sidebar-section="${sectionName}"]`)?.focus();
  }, name);
  await browser.keys("Enter");
}

async function switchGraph(browser, graph) {
  await browser.$(".graph-switch-btn").click();
  await browser.$(".graph-switch-menu").waitForExist({ timeout: 5000 });
  const clicked = await browser.execute((target) => {
    const row = [...document.querySelectorAll(".graph-switch-row")].find((node) => node.getAttribute("title") === target);
    row?.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0 }));
    return Boolean(row);
  }, graph);
  if (!clicked) throw new Error(`known graph row missing for ${graph}`);
  const expectedName = path.basename(graph);
  try {
    await browser.waitUntil(async () => browser.execute((name) =>
      document.querySelector(".graph-switch-name")?.textContent?.trim() === name,
    expectedName), {
      timeout: 40_000, timeoutMsg: `${expectedName} graph did not become active`,
    });
  } catch (error) {
    const state = await browser.execute(() => ({
      graphName: document.querySelector(".graph-switch-name")?.textContent ?? null,
      toasts: [...document.querySelectorAll(".toast")].map((node) => node.textContent),
      title: document.querySelector("h1.page-title")?.textContent ?? null,
    })).catch(() => ({ graphName: null, toasts: ["webview unavailable"], title: null }));
    throw new Error(`${String(error)}; graph UI state ${JSON.stringify(state)}`);
  }
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
}

await withApp(GRAPH_A, 0, async (browser) => {
  await navigatePage(browser, "Recent A");
  await (await expectExpanded(browser, "favorites", true)).click();
  await expectExpanded(browser, "recent", true);
  await activateSectionWithKeyboard(browser, "recent");
  await expectExpanded(browser, "favorites", false);
  await expectExpanded(browser, "recent", false);

  await switchGraph(browser, GRAPH_B);
  await expectExpanded(browser, "favorites", true);
  await navigatePage(browser, "Recent B");
  await expectExpanded(browser, "recent", true);

  await switchGraph(browser, GRAPH_A);
  await expectExpanded(browser, "favorites", false);
  await navigatePage(browser, "Recent A");
  await expectExpanded(browser, "recent", false);
});

// A new native process must restore graph A's two independent preferences.
await withApp(GRAPH_A, 1, async (browser) => {
  await expectExpanded(browser, "favorites", false);
  const recent = await browser.$('[data-sidebar-section="recent"]');
  if (!(await recent.isExisting())) await navigatePage(browser, "Recent A");
  await expectExpanded(browser, "recent", false);
});

console.log("PASS: Favorites/Recent disclosures persisted per graph across switches and a native restart");

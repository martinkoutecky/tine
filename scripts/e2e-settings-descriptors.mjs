// GH #866 / L4-04: real Settings search -> theme/settings row, on a routed page.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { setTimeout as poll } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

if (process.argv.includes("--help")) {
  console.log('Usage: xvfb-run -a -s "-screen 0 1920x1080x24" node scripts/e2e-settings-descriptors.mjs');
  process.exit(0);
}
ensurePrivateSessionBus();
assert.ok(process.env.DISPLAY, "Run under xvfb-run");
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const artifacts = path.resolve("artifacts/og-settings-descriptors-866/native");
fs.mkdirSync(artifacts, { recursive: true });
const fixture = fs.mkdtempSync(path.join(artifacts, "journey-"));
const graph = path.join(fixture, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(fixture, dir));
const note = "- Settings search acceptance\n";
fs.writeFileSync(path.join(graph, "pages/Discovery.md"), note);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const installed = { schemaVersion: 1, apiVersion: "0.2", id: "page.tine.theme.discovery", name: "Orchard dusk", version: "1.0.0",
  description: "Apricot paper and ink", author: "Tine", license: "MIT", source: "https://example.invalid/theme",
  modes: { light: { "--ls-primary-background-color": "#fefefe" } }, screenshots: [] };
const settingsFile = path.join(fixture, "data", APP_ID, "tine-settings.json");
fs.mkdirSync(path.dirname(settingsFile), { recursive: true });
fs.writeFileSync(settingsFile, JSON.stringify({ last_graph_path: graph, "theme.packages.v1": JSON.stringify([installed]) }));
const home = accountHomeFixture(fixture, fixture);
const env = { ...home.env, HOME: path.join(fixture, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(fixture, "data"), XDG_CONFIG_HOME: path.join(fixture, "config"), XDG_CACHE_HOME: path.join(fixture, "cache"), XDG_STATE_HOME: path.join(fixture, "state"),
  GDK_BACKEND: "x11", WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1" };
const port = Number(process.env.E2E_DRIVER_PORT || 4983);
const log = fs.openSync(path.join(fixture, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"), journey: "Settings descriptor discovery",
  expected: "theme names/descriptions and localized setting labels land on their own rows", steps: [] };
let browser;
async function findAndOpen(query, id) {
  await browser.$(".settings-search-input").setValue(query);
  await browser.waitUntil(() => browser.execute((id) => [...document.querySelectorAll(".settings-search-result")]
    .some(row => row.getAttribute("data-setting-target") === id), id), { timeout: 10_000, timeoutMsg: `No result for ${query} (${id})` });
  await browser.execute(id => [...document.querySelectorAll(".settings-search-result")]
    .find(row => row.getAttribute("data-setting-target") === id).click(), id);
  await browser.waitUntil(() => browser.execute(id => {
    const row = [...document.querySelectorAll(".settings-modal [data-setting-id]")].find(row => row.getAttribute("data-setting-id") === id);
    return row?.classList.contains("settings-jump-flash") && !document.querySelector(".settings-search-input").value;
  }, id), { timeout: 10_000, timeoutMsg: `Result did not land on ${id}` });
  const visible = await browser.execute(id => {
    const row = [...document.querySelectorAll(".settings-modal [data-setting-id]")].find(row => row.getAttribute("data-setting-id") === id);
    const bounds = row.getBoundingClientRect(), pane = document.querySelector(".settings-pane-body").getBoundingClientRect();
    return bounds.bottom > pane.top && bounds.top < pane.bottom;
  }, id);
  assert.ok(visible, `${query}: target is in the Settings viewport`);
  proof.steps.push({ query, id });
  console.log(`PASS ${query} -> ${id}`);
}
try {
  const deadline = Date.now() + 10_000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    assert.ok(Date.now() < deadline, "driver readiness"); await poll(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1280, 900);
  await openPageByName(browser, "Discovery");
  await assertAppLease(home);
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-search-input").waitForDisplayed({ timeout: 10_000 });
  const themeId = "theme.installed.page.tine.theme.discovery@1.0.0";
  for (const locale of ["en", "zh-CN"]) {
    if (locale === "zh-CN") {
      await findAndOpen("Language", "settings-language-label");
      await browser.$('[data-setting-id="settings-language-label"] select').selectByAttribute("value", locale);
      await browser.waitUntil(() => browser.execute(() => document.querySelector('[data-setting-id="settings-language-label"] .settings-label')?.textContent === "语言"), { timeout: 10_000 });
    }
    await findAndOpen("Orchard dusk", themeId);
    await findAndOpen("Apricot paper and ink", themeId);
    await findAndOpen("Nord", "theme.gallery.nord");
    for (const id of ["settings-ui-wide-mode", "settings-ui-journal-date-format", "content-width-fields-standard-page-width"]) {
      // Obtain the actual translated label by visiting the row once, then type
      // that label into search. English fallback terms open it from either locale.
      const english = { "settings-ui-wide-mode": "Wide mode", "settings-ui-journal-date-format": "Journal date format", "content-width-fields-standard-page-width": "Standard page width" }[id];
      await findAndOpen(english, id);
      const label = await browser.execute(id => document.querySelector(`[data-setting-id="${id}"]`).getAttribute("data-setting-label"), id);
      await findAndOpen(label, id);
    }
    execFileSync("import", ["-display", env.DISPLAY, "-window", "root", path.join(fixture, `settings-${locale}.png`)], { env, timeout: 10_000 });
  }
  assert.equal(fs.readFileSync(path.join(graph, "pages/Discovery.md"), "utf8"), note);
  execFileSync("import", ["-display", env.DISPLAY, "-window", "root", path.join(fixture, "settings.png")], { env, timeout: 10_000 });
  fs.writeFileSync(path.join(fixture, "proof.json"), JSON.stringify(proof, null, 2));
} catch (error) {
  try { execFileSync("import", ["-display", env.DISPLAY, "-window", "root", path.join(fixture, "failure.png")], { env, timeout: 10_000 }); } catch {}
  fs.writeFileSync(path.join(fixture, "failure.json"), JSON.stringify({ ...proof, observed: String(error), classification: "ambiguous" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  home.close?.();
  home.verify();
}

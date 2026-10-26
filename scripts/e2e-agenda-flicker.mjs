// Discussion #617: observe the actual journal during in-place graph switches.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

ensurePrivateSessionBus();
const out = path.resolve(process.env.OUT ?? `artifacts/agenda-flicker-617/native-${Date.now()}`);
const commit = execFileSync("rtk", ["proxy", "git", "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const alpha = `${out}/alpha`, beta = `${out}/beta`;
const today = new Date();
const iso = [today.getFullYear(), String(today.getMonth() + 1).padStart(2, "0"), String(today.getDate()).padStart(2, "0")].join("-");
for (const [graph, name] of [[alpha, "ALPHA"], [beta, "BETA"]]) {
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${graph}/${dir}`, { recursive: true });
  fs.writeFileSync(`${graph}/journals/${iso.replaceAll("-", "_")}.md`, `- ${name}_JOURNAL\n`);
  fs.writeFileSync(`${graph}/logseq/config.edn`, '{:default-queries {:journals [{:title "Default tasks" :query (task TODO)}]}}\n');
}
fs.writeFileSync(`${alpha}/pages/Tasks.md`, `- TODO Scheduled fixture\n  SCHEDULED: <${iso} ${today.toLocaleDateString("en-US", { weekday: "short" })}>\n`);
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${out}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(`${out}/xdg/data/${APP_ID}`, { recursive: true });
fs.writeFileSync(`${out}/xdg/data/${APP_ID}/tine-settings.json`, JSON.stringify({
  known_graphs: [{ name: "alpha", path: alpha }, { name: "beta", path: beta }], last_graph_path: alpha,
}));
const env = { ...process.env, TINE_GRAPH: alpha, XDG_DATA_HOME: `${out}/xdg/data`, XDG_CONFIG_HOME: `${out}/xdg/config`,
  XDG_CACHE_HOME: `${out}/xdg/cache`, WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const driverPort = Number(process.env.E2E_DRIVER_PORT ?? 4668), nativePort = Number(process.env.E2E_NATIVE_PORT ?? 4669);
const driverLog = fs.openSync(`${out}/driver.log`, "w");
const driver = spawn(process.env.TAURI_DRIVER ?? `${process.env.CARGO_HOME}/bin/tauri-driver`,
  ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER ?? "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", driverLog, driverLog], detached: true });
let browser, phase = "launch";
const proof = { commit, observations: [] };
async function chooseGraph(graph, name) {
  phase = `switch to ${name}`;
  await browser.$(".graph-switch-btn").click();
  await browser.$(".graph-switch-row").waitForExist({ timeout: 5000 });
  assert.ok(await browser.execute((wanted) => {
    const row = [...document.querySelectorAll(".graph-switch-row")].find(node => node.getAttribute("title") === wanted);
    row?.click(); return !!row;
  }, graph), "known graph is offered");
  await browser.waitUntil(() => browser.execute((wanted) => document.querySelector(".graph-switch-name")?.textContent.trim() === wanted
    && document.querySelector(".page-blocks")?.textContent.includes(`${wanted.toUpperCase()}_JOURNAL`), name), { timeout: 30_000 });
}
try {
  await sleep(1500);
  console.log("Creating native WebDriver session");
  browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error", connectionRetryCount: 1,
    connectionRetryTimeout: 20_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: path.resolve(process.env.TINE_APP ?? "target/debug/tine") } } });
  await ensureMainWindow(browser);
  console.log("Waiting for the initial agenda and default query");
  phase = "initial nonempty journal";
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".agenda-block .query-block")?.textContent.includes("Scheduled fixture")
    && document.querySelector(".today-queries .query-block")?.textContent.includes("Scheduled fixture")), { timeout: 60_000 });
  await browser.execute(() => {
    window.agendaSamples = [];
    const sample = () => {
      const graph = document.querySelector(".graph-switch-name")?.textContent.trim();
      const journal = document.querySelector(".page-blocks")?.textContent ?? "";
      const sections = [...document.querySelectorAll(".agenda-block .query-block, .today-queries .query-block")].map(node => node.textContent);
      window.agendaSamples.push({ graph, ownJournal: journal.includes(`${graph?.toUpperCase()}_JOURNAL`), sections });
    };
    window.agendaObserver = new MutationObserver(sample);
    window.agendaObserver.observe(document.body, { childList: true, subtree: true, characterData: true });
    sample();
  });
  await chooseGraph(beta, "beta");
  console.log("Empty graph journal opened");
  await browser.$(".today-sections-customize").waitForExist({ timeout: 10_000 });
  // No screenshot here: WebKitWebDriver screenshots stall under the gate's
  // openbox session; the DOM samples below are the evidence.
  await sleep(1000);
  await chooseGraph(alpha, "alpha");
  console.log("Nonempty graph journal reopened");
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".agenda-block .query-block")?.textContent.includes("Scheduled fixture")
    && document.querySelector(".today-queries .query-block")?.textContent.includes("Scheduled fixture")), { timeout: 30_000 });
  const samples = await browser.execute(() => { window.agendaObserver.disconnect(); return window.agendaSamples; });
  fs.writeFileSync(`${out}/samples.json`, JSON.stringify(samples, null, 2));
  const empty = samples.filter(sample => sample.graph === "beta" && sample.ownJournal);
  assert.ok(empty.length > 0, "the observer sampled the empty replacement journal");
  assert.ok(empty.every(sample => sample.sections.length === 0), "empty replacement journal never shows query sections");
  assert.ok(samples.every(sample => sample.sections.every(text => !text.includes("Loading query results"))), "sections with results appear ready");
  proof.observations.push(`PASS: ${samples.length} DOM mutation samples; ${empty.length} on beta's own empty journal; no transient section or loading placeholder; alpha agenda and default queries return ready.`);
  console.log(proof.observations[0]);
} catch (error) {
  proof.failure = { phase, expected: "empty journal has no transient query section; nonempty sections appear ready", observed: String(error), classification: "ambiguous" };
  if (browser) {
    await browser.saveScreenshot(`${out}/failure.png`).catch(() => {});
    proof.dom = await browser.execute(() => ({ text: document.body.innerText, html: document.querySelector(".today-queries")?.outerHTML })).catch(() => null);
  }
  throw error;
} finally {
  fs.writeFileSync(`${out}/proof.json`, JSON.stringify(proof, null, 2));
  if (browser) await browser.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(driverLog);
}

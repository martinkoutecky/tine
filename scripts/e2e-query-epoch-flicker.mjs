// UI-QUERY-RESULT-EPOCH-FLICKER (Martin 2026-10-15): adding a block on today's
// journal dropped the Scheduled & Deadline results to read-only stand-ins (no
// children, dates or breadcrumbs) until the query reran about a second later.
// Real app: a scheduled task with a child shows under today; Enter in today's
// journal must keep that child visible on every animation frame.
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openJournals } from "./lib/e2e-navigation.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/query-epoch-flicker");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const day = offset => { const d = new Date(); d.setDate(d.getDate() + offset); return d; };
const iso = d => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
const soon = day(2);
const weekday = soon.toLocaleDateString("en-US", { weekday: "short" });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), `{:journal/page-title-format "yyyy-MM-dd"}`);
fs.writeFileSync(path.join(graph, "journals", `${iso(day(0)).replaceAll("-", "_")}.md`), "- journal-entry\n");
fs.writeFileSync(path.join(graph, "pages/Tasks.md"),
  `- TODO agenda-task\n  SCHEDULED: <${iso(soon)} ${weekday}>\n\t- live-child-marker\n`);
const base = execFileSync("rtk", ["proxy", "git", "rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4796);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
let stage = "startup";
const agendaShowsChild = () => browser.execute(() => !!document.querySelector(".agenda-block")?.textContent.includes("live-child-marker"));
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/debug/tine") } } });
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".nav-item")].some(item => item.textContent?.trim() === "Journals")),
    { timeout: 60_000, timeoutMsg: "app never showed its navigation" });
  await openJournals(browser);
  stage = "agenda shows the live result with its child";
  await browser.waitUntil(agendaShowsChild, { timeout: 30_000, timeoutMsg: "agenda never showed the scheduled task's child" });
  // Let the first answer settle so the sample below sees only the edit's effect.
  await browser.pause(1500);
  stage = "edit today's journal";
  const entry = await browser.$("//div[contains(@class,'ls-block')][not(ancestor::*[contains(@class,'agenda-block')])]//*[contains(text(),'journal-entry')]");
  await entry.click();
  await browser.waitUntil(() => browser.execute(() => document.activeElement?.tagName === "TEXTAREA"), { timeout: 10_000, timeoutMsg: "journal block did not enter editing" });
  await browser.keys("End");
  await browser.execute(() => {
    const samples = [];
    let frame = 0;
    const sample = () => {
      const agenda = document.querySelector(".agenda-block");
      samples.push({ t: Math.round(performance.now()), child: !!agenda?.textContent.includes("live-child-marker"),
        task: !!agenda?.textContent.includes("agenda-task") });
      frame = requestAnimationFrame(sample);
    };
    sample();
    window.__epochFlicker = { stop: () => { cancelAnimationFrame(frame); sample(); return samples; } };
  });
  await browser.keys("Enter");
  await browser.keys("new-block");
  await browser.pause(3000);
  const samples = await browser.execute(() => window.__epochFlicker.stop());
  fs.writeFileSync(path.join(run, "samples.json"), JSON.stringify(samples));
  const lost = samples.filter(s => s.task && !s.child);
  stage = "agenda child stays visible through Enter";
  if (!samples.length || lost.length) {
    throw new Error(`child hidden on ${lost.length}/${samples.length} frames (first at ${lost[0]?.t}, last at ${lost.at(-1)?.t})`);
  }
  if (!await agendaShowsChild()) throw new Error("child missing after the edit settled");
  await browser.saveScreenshot(path.join(run, "passed.png"));
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify({ base, app: process.env.TINE_APP ?? null, frames: samples.length }, null, 2));
  console.log(`PASS query-epoch-flicker: child visible on all ${samples.length} frames (${run})`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(run, "failure.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${base}\nApp: ${process.env.TINE_APP ?? "target/debug/tine"}\nJourney: ${stage}\nExpected: agenda result keeps its child on every frame after Enter in today's journal\nObserved: ${error.stack}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}

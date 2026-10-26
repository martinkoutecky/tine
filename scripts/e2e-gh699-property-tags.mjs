// GH #699: a routed page's separately rendered property tags open their own
// pages; those pages show the source in Tagged Pages / Linked References.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = process.cwd();
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const OUT = path.resolve(process.env.E2E_ARTIFACT_DIR || "artifacts/gh699/native");
fs.mkdirSync(OUT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(OUT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
const sources = { Space: "#eng #exec #founder", Comma: "#eng,#exec,#founder", CommaSpace: "#eng, #exec, #founder" };
for (const [name, value] of Object.entries(sources)) fs.writeFileSync(path.join(GRAPH, `pages/${name}.md`), `tags:: ${value}\n\n- Source body\n`);
fs.writeFileSync(path.join(GRAPH, "pages/eng.md"), "- Engineering destination\n");
const initial = Object.fromEntries(Object.keys(sources).map(name => [name, fs.readFileSync(path.join(GRAPH, `pages/${name}.md`), "utf8")]));
const env = { ...process.env, TINE_GRAPH: GRAPH, XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4799);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"), ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: process.env.E2E_TESTED_COMMIT, binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), steps: [] };
let browser;
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  for (const name of Object.keys(sources)) {
    await openPageByName(browser, name);
    await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".main-content .prop-row .page-ref")].map(a => a.textContent).join("|") === "eng|exec|founder"), { timeout: 15_000, timeoutMsg: `${name}: three separately rendered tags` });
    const links = await browser.$$(".main-content .prop-row .page-ref");
    await links[0].click();
    await browser.waitUntil(() => browser.execute(() => document.querySelector(".main-content")?.textContent.includes("Engineering destination")), { timeout: 15_000, timeoutMsg: `${name}: clicking eng opens its own page` });
    await browser.waitUntil(() => browser.execute((source) => [...document.querySelectorAll(".main-content .linked-references:not(.tagged-pages) .reference-page")].some(page => page.textContent === source), name), { timeout: 15_000, timeoutMsg: `${name}: eng backlink includes source` });
    await browser.waitUntil(() => browser.execute((source) => document.querySelector(".main-content .tagged-pages")?.textContent.includes(source), name), { timeout: 15_000, timeoutMsg: `${name}: eng tagged pages includes source` });
    proof.steps.push(`${name}: three tags; eng click routes to destination with source backlink`);
  }
  fs.writeFileSync(path.join(TMP, "destination.html"), await browser.execute(() => document.body.innerHTML));
  for (const [name, text] of Object.entries(initial)) {
    if (fs.readFileSync(path.join(GRAPH, `pages/${name}.md`), "utf8") !== text) throw new Error(`${name}: browsing changed source file`);
  }
  proof.steps.push("all source files remain byte-identical after browsing");
  console.log(`PASS ${JSON.stringify(proof)}`);
} catch (error) {
  proof.error = String(error);
  proof.classification = "ambiguous; inspect captured DOM and driver log";
  if (browser) {
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), JSON.stringify(proof, null, 2));
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}

#!/usr/bin/env node
// Disposable native Exports overview → reviewed Replace → durable leaf update.
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/folder-exports");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["graph/pages", "graph/journals", "graph/logseq", "graph/published-queries/saved-leaf/app", "data", "config", "cache"]) fs.mkdirSync(path.join(out, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
fs.writeFileSync(path.join(graph, "pages/Project.md"), "- Updated folder sentinel\n");
fs.writeFileSync(path.join(graph, "pages/Exports.md"), "tine/exports:: true\npublic:: false\n\n- {{query (or (property export) (property tine.share))}}\n- {{tine-query @page and name = 'Project'}}\n  export:: saved-leaf\n");
const leaf = path.join(graph, "published-queries/saved-leaf");
fs.writeFileSync(path.join(leaf, "previous.txt"), "Previous folder sentinel");
fs.writeFileSync(path.join(leaf, "app/snapshot.json"), JSON.stringify({ pages: [{ name: "Old", path: "pages/Old.md" }] }));
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const port = Number(process.env.E2E_DRIVER_PORT || 4896);
const env = { ...process.env, TINE_GRAPH: graph, TINE_QUICK_SHARE: "0",
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"), journey: "Exports overview Re-export, then Update all with sharing disabled", expected: "Reviewed Replace updates the recorded leaf only after confirmation, preserves the old export, and Update all succeeds without quick share", steps: [], outcome: "pending" };
try {
  const deadline = Date.now() + 15_000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error("driver startup failed");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser); await browser.setWindowSize(1400, 1000);
  console.log("opened application");
  await browser.$(".page-title").waitForExist({ timeout: 30_000 }); await openPageByName(browser, "Exports");
  console.log("opened Exports");
  await browser.$(".query-results-region .folder-export-update").waitForExist({ timeout: 20_000 });
  await browser.execute(() => document.querySelector(".query-results-region .folder-export-update").click());
  await browser.$(".folder-export-membership").waitForExist({ timeout: 20_000 });
  if (!await browser.execute(() => document.querySelector('.query-export-modal input[type="radio"]')?.checked)) throw new Error("Replace was not preset");
  if (!fs.existsSync(path.join(leaf, "previous.txt"))) throw new Error("leaf changed before confirmation");
  proof.steps.push("Replace preset; old leaf unchanged before confirm");
  await browser.execute(() => document.querySelector(".query-export-modal .export-btn-primary").click());
  await browser.waitUntil(() => fs.readFileSync(path.join(leaf, "app/snapshot.json"), "utf8").includes("Updated folder sentinel"), { timeout: 30_000 });
  if (fs.existsSync(path.join(leaf, "previous.txt"))) throw new Error("old leaf was not replaced");
  const recovery = path.join(graph, "logseq/.tine-trash/conflicts");
  if (!fs.readdirSync(recovery, { recursive: true }).some(name => String(name).endsWith("previous.txt"))) throw new Error("previous export not retained in recovery");
  proof.steps.push("snapshot contains edited source; previous leaf preserved in recovery");
  await browser.$(".quick-share-all").click();
  await browser.$(".quick-share-all-confirm").waitForEnabled({ timeout: 20_000 });
  await browser.execute(() => document.querySelector(".quick-share-all-confirm").click());
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll('.quick-share-all-entry [role="status"]')].some(node => node.textContent.includes("Exported"))), { timeout: 30_000 });
  fs.writeFileSync(path.join(out, "updated-ui.txt"), await browser.execute(() => document.body.innerText));
  proof.steps.push("Update all shows the folder publication outcome");
  proof.outcome = "passed: reviewed Replace, snapshot updated, old leaf recovered, Update all completed with sharing disabled";
} catch (error) {
  proof.outcome = "failed"; proof.error = String(error);
  proof.classification = "ambiguous";
  if (browser) {
    const state = await browser.execute(() => ({ url: location.href, text: document.body.innerText, html: document.body.innerHTML })).catch(failure => ({ error: String(failure) }));
    fs.writeFileSync(path.join(out, "failure-state.json"), JSON.stringify(state, null, 2));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  if (browser) await browser.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  console.log(path.join(out, "proof.json"));
}

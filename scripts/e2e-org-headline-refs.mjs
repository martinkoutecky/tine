// Kalem parity: parser-owned Org headline tags reach both visible read surfaces.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver");
const ARTIFACT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/k3/native"));
fs.mkdirSync(ARTIFACT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
const SOURCE = path.join(GRAPH, "pages/Source.org");
const INITIAL = "* Headline ref witness :Plugins:\n";
fs.writeFileSync(SOURCE, INITIAL);
fs.writeFileSync(path.join(GRAPH, "pages/Plugins.md"), "- Target\n");
fs.writeFileSync(path.join(GRAPH, "pages/Queries.md"), "- {{query [[Plugins]]}}\n");
const now = new Date();
const day = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${day}.md`), "- Open [[Queries]]\n");
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const port = Number(process.env.E2E_DRIVER_PORT || 4886);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { app: APP, binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), graph: GRAPH, steps: [] };
async function visible(selector, present) {
  await browser.waitUntil(() => browser.execute((scope, expected) => {
    const node = document.querySelector(scope);
    if (!node && scope.includes("query-block")) return false;
    node?.scrollIntoView({ block: "center" });
    return (node?.textContent.includes("Headline ref witness") ?? false) === expected;
  }, selector, present), { timeout: 30_000, interval: 200, timeoutMsg: `${selector}: expected headline witness present=${present}` });
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Queries");
  await visible(".main-content .query-block", true);
  proof.steps.push("Org headline appears in rendered page-reference query");
  await openPageByName(browser, "Plugins");
  await visible(".main-content .linked-references", true);
  proof.steps.push("Org headline appears in Linked References");
  // External delivery invalidates the cached projection and both read answers.
  fs.writeFileSync(SOURCE, "* Headline ref witness :Different:\n");
  await visible(".main-content .linked-references", false);
  await openPageByName(browser, "Queries");
  await visible(".main-content .query-block", false);
  proof.steps.push("changing the headline tag removes the old query and backlink rows live");
  fs.writeFileSync(SOURCE, INITIAL);
  await visible(".main-content .query-block", true);
  proof.steps.push("restoring the tag restores the query row live");
  fs.writeFileSync(path.join(TMP, "final-dom.html"), await browser.execute(() => document.body.innerHTML));
  console.log(`PASS ${JSON.stringify(proof)}`);
} catch (error) {
  proof.error = String(error);
  proof.classification = "ambiguous; inspect DOM and driver log";
  if (browser) fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  throw error;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}

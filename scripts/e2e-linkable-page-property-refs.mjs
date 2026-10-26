// Kalem parity: configured page-property atoms reach both visible read surfaces.
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
const ARTIFACT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/k2/native"));
fs.mkdirSync(ARTIFACT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{:property/separated-by-commas #{:related}}\n");
const SOURCE = path.join(GRAPH, "pages/Source.md");
const INITIAL = "related:: Plugins，Witness\naad:: val12\n\n- Body\n";
fs.writeFileSync(SOURCE, INITIAL);
fs.writeFileSync(path.join(GRAPH, "pages/Plugins.md"), "- Target\n");
fs.writeFileSync(path.join(GRAPH, "pages/Witness.md"), "- Second target\n");
fs.writeFileSync(path.join(GRAPH, "pages/Queries.md"), "- {{query [[Plugins]]}}\n");
const now = new Date();
const day = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${day}.md`), "- Open [[Queries]]\n");
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const port = Number(process.env.E2E_DRIVER_PORT || 4896);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { app: APP, testedCommit: process.env.E2E_TESTED_COMMIT, binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), graph: GRAPH, steps: [] };
async function visible(selector, present) {
  await browser.waitUntil(() => browser.execute((scope, expected) => {
    const node = document.querySelector(scope);
    if (!node && scope.includes("query-block")) return false;
    node?.scrollIntoView({ block: "center" });
    return (node?.textContent.includes("related") ?? false) === expected;
  }, selector, present), { timeout: 30_000, interval: 200, timeoutMsg: `${selector}: expected headline witness present=${present}` });
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Queries");
  await visible(".main-content .query-block", true);
  proof.steps.push("configured bare page-property atom appears in rendered page-reference query");
  await openPageByName(browser, "Plugins");
  await visible(".main-content .linked-references", true);
  proof.steps.push("configured bare page-property atom appears in Linked References");
  // GH #753: the literal report journey, followed by another group for Source.
  // Compare the read-only property markup within this run, allowing the edited
  // value to change. This checks renderer consistency without pinning a design.
  const rowMarkup = () => browser.execute(() => {
    const row = document.querySelector(".main-content .linked-references .page-property-reference .block-content");
    return row?.innerHTML.replaceAll("val13", "val12") ?? null;
  });
  const before = await rowMarkup();
  if (!before) throw new Error("GH753: missing initial page-property row");
  await browser.$(".main-content .linked-references .page-property-reference-row:last-child .prop-value").click();
  const editor = await browser.$(".main-content .linked-references textarea");
  await editor.waitForDisplayed({ timeout: 10_000 });
  await editor.click();
  await browser.execute(() => {
    const textarea = document.querySelector(".main-content .linked-references textarea");
    const start = textarea.value.indexOf("val12");
    if (start < 0) throw new Error("GH753: initial property value missing in editor");
    textarea.setSelectionRange(start, start + "val12".length);
  });
  await browser.keys("val13");
  await browser.$(".main-content .linked-references .reference-page").click();
  await browser.waitUntil(() => fs.readFileSync(SOURCE, "utf8").includes("aad:: val13"), {
    timeout: 15_000, timeoutMsg: "GH753: edited property was not saved to the source file",
  });
  await openPageByName(browser, "Plugins");
  await visible(".main-content .linked-references", true);
  await browser.waitUntil(async () => (await rowMarkup()) === before, {
    timeout: 15_000, timeoutMsg: "GH753: property row changed appearance after clicking the group title",
  });
  await openPageByName(browser, "Witness");
  await visible(".main-content .linked-references", true);
  await browser.waitUntil(async () => (await rowMarkup()) === before, {
    timeout: 15_000, timeoutMsg: "GH753: another group for Source changed appearance after editing",
  });
  proof.steps.push("GH753: click property, edit val12 to val13, click group title; same read-only markup on both targets and edit saved on disk");
  fs.writeFileSync(path.join(TMP, "gh753-after-edit-dom.html"), await browser.execute(() => document.body.innerHTML));
  await openPageByName(browser, "Plugins");
  await visible(".main-content .linked-references", true);
  // External delivery invalidates the cached projection and both read answers.
  fs.writeFileSync(SOURCE, "related:: Different，Witness\n\n- Body\n");
  await visible(".main-content .linked-references", false);
  await openPageByName(browser, "Queries");
  await visible(".main-content .query-block", false);
  proof.steps.push("changing the property atom removes the old query and backlink rows live");
  fs.writeFileSync(SOURCE, INITIAL);
  await visible(".main-content .query-block", true);
  proof.steps.push("restoring the property restores the query row live");
  fs.writeFileSync(path.join(TMP, "final-dom.html"), await browser.execute(() => document.body.innerHTML));
  console.log(`PASS ${JSON.stringify(proof)}`);
} catch (error) {
  proof.error = String(error);
  proof.classification = "ambiguous; inspect DOM and driver log";
  if (browser) {
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}

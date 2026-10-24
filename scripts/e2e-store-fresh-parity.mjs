// Store parity at the native consumer: external journal arrivals, linked
// references, and a config rebuild while the canonical icon page is held.
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/og-store-fresh-parity/native"));
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "run-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
const files = {
  "pages/a.md": "- target [[Delta]]\n",
  "pages/Delta.md": "icon:: 🚀\n\n- canonical icon page\n",
  "pages/twin.md": "icon:: 📚\ntitle:: Delta\n\n- competing title page\n",
  "journals/2026_10_02.md": "- canonical journal reference [[a]]\n",
  "logseq/config.edn": "{}\n",
};
for (const [rel, bytes] of Object.entries(files)) fs.writeFileSync(path.join(graph, rel), bytes);
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const port = Number(process.env.E2E_DRIVER_PORT || 4816);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log] });
const passed = [];
let browser;
let bindingGeneration;
let stage = "launch";
const assert = (ok, message) => { if (!ok) throw new Error(message); };
const invoke = async (command, args = {}) => {
  const result = await browser.executeAsync((command, args, generation, done) => {
    window.__TAURI_INTERNALS__.invoke(command, { ...args, bindingGeneration: generation })
      .then(value => done({ ok: true, value }), error => done({ ok: false, error: String(error) }));
  }, command, args, bindingGeneration);
  assert(result.ok, `${command}: ${result.error}`);
  return result.value;
};
const inventory = () => invoke("page_inventory");
const icons = () => invoke("page_icons", { names: ["Delta"] });
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/debug/tine") } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1100, 900);
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  // Reading this window's existing binding is the native harness's standard
  // lease acquisition; it does not switch or reopen the graph.
  const binding = await browser.executeAsync((root, done) => {
    window.__TAURI_INTERNALS__.invoke("load_graph", { path: root })
      .then(done, error => done({ error: String(error) }));
  }, graph);
  assert(binding.kind === "already_current", `unexpected graph binding: ${JSON.stringify(binding)}`);
  bindingGeneration = binding.binding_generation;
  await openPageByName(browser, "a");
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".reference-group")]
    .some(node => node.textContent.includes("canonical journal reference"))), { timeout: 20_000 });

  stage = "a live duplicate journal stays detectable and leaves linked references unchanged";
  fs.writeFileSync(path.join(graph, "journals/2026-10-02.md"), "- losing journal reference [[a]]\n");
  await browser.waitUntil(async () => (await inventory()).entries.some(entry => entry.is_journal && entry.target.others?.length === 1),
    { timeout: 20_000, timeoutMsg: "the watcher did not publish the duplicate claimant" });
  await openPageByName(browser, "Delta");
  await openPageByName(browser, "a");
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".reference-group")]
    .some(node => node.textContent.includes("canonical journal reference"))), { timeout: 20_000 });
  const references = await browser.execute(() => [...document.querySelectorAll(".reference-group")].map(node => node.textContent).join("\n"));
  assert(!references.includes("losing journal reference"), `a duplicate journal leaked into linked references: ${references}`);
  assert(fs.readFileSync(path.join(graph, "journals/2026-10-02.md"), "utf8") === "- losing journal reference [[a]]\n", "the stray was changed");
  passed.push(stage);

  stage = "a held canonical page keeps its icon across a real config reload";
  await openPageByName(browser, "Delta");
  assert((await icons()).Delta === "🚀", "the initial icon did not belong to the canonical file");
  fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:journal/file-name-format \"yyyy-MM-dd\"}\n");
  // The configured filename now prefers the hyphenated journal. A revision
  // alone could belong to an unrelated pending watcher publication.
  await browser.waitUntil(async () => (await inventory()).entries.some(entry => entry.is_journal
    && entry.target.id === "journals/2026-10-02.md"
    && entry.target.others?.includes("journals/2026_10_02.md")),
  { timeout: 20_000, timeoutMsg: "the configured journal claimant order did not change" });
  assert((await icons()).Delta === "🚀", "the native icon consumer switched to the losing title claimant");
  assert(await browser.execute(() => {
    const icon = document.querySelector("h1.page-title .page-icon");
    return icon?.textContent.includes("🚀") || [...(icon?.querySelectorAll("[alt], [aria-label]") ?? [])]
      .some(node => (node.getAttribute("alt") || node.getAttribute("aria-label")) === "🚀");
  }), "the visible held page lost its icon");
  for (const rel of ["pages/a.md", "pages/Delta.md", "pages/twin.md", "journals/2026_10_02.md"]) {
    assert(fs.readFileSync(path.join(graph, rel), "utf8") === files[rel], `an unrelated graph file changed: ${rel}`);
  }
  passed.push(stage);
  fs.writeFileSync(path.join(run, "pass-dom.html"), await browser.execute(() => document.documentElement.outerHTML));
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "pass", testedCommit, passed, graph }, null, 2));
  console.log(`PASS ${passed.join("; ")}`);
} catch (error) {
  if (browser) {
    try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.execute(() => document.documentElement.outerHTML)); } catch {}
  }
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "fail", testedCommit, classification: "ambiguous", expected: stage, observed: String(error), passed, graph }, null, 2));
  console.error(`FAIL ${stage}: ${error}`);
  process.exitCode = 1;
} finally {
  await browser?.deleteSession().catch(() => {});
  driver.kill();
  fs.closeSync(log);
  console.log(`Artifacts: ${run}`);
}

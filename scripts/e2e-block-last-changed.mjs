// GH #831: real bullet hovers on a routed synthetic git page with fixed dates.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { setTimeout as poll } from "node:timers/promises";
import { remote } from "webdriverio";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";

const run = path.resolve(`target/artifacts/og-block-times-831/native-${Date.now()}`);
const graph = path.join(run, "graph");
for (const dir of ["graph/pages", "graph/journals", "graph/logseq", "home", "data", "config", "cache", "state"])
  fs.mkdirSync(path.join(run, dir), { recursive: true });
const account = installAccountHome(run);
const env = { ...process.env, HOME: path.join(run, "home"), XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"),
  TINE_GRAPH: graph, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const git = (...args) => execFileSync("git", ["-C", graph, ...args], { env, encoding: "utf8" }).trim();
const page = path.join(graph, "pages/History.md");
const text = local => `- Untouched since creation\n- Edited in the second commit\n- ${local}\n`;
git("init", "-q", "-b", "main");
git("config", "user.name", "Fixture writer"); git("config", "user.email", "fixture@example.invalid");
function commit(message, date) {
  git("add", "-A");
  execFileSync("git", ["-C", graph, "commit", "-q", "-m", message], {
    env: { ...env, GIT_AUTHOR_DATE: date, GIT_COMMITTER_DATE: date },
  });
}
fs.writeFileSync(page, "- Untouched since creation\n- Before the second commit\n- Before local editing\n");
commit("creation", "2024-01-10T10:00:00Z");
fs.writeFileSync(page, text("Before local editing"));
commit("edit the second block", "2024-02-11T11:00:00Z");
fs.writeFileSync(path.join(graph, "pages/Other.md"), "- Unrelated third commit\n");
commit("unrelated HEAD", "2024-03-12T12:00:00Z");
fs.writeFileSync(page, text("Changed locally without a commit"));
const before = fs.readFileSync(page, "utf8");
const headBefore = git("rev-parse", "HEAD");
const app = process.env.TINE_APP || path.resolve("target/release/tine");
assert.ok(fs.existsSync(app), `Build the literal binary first: ${app}`);
const port = Number(process.env.E2E_DRIVER_PORT || 4831);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4832);
const target = await startWebdriverApplication(app, env, nativePort);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort,
  process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env: target.env, stdio: ["ignore", log, log], detached: true });
let browser;
const evidence = { testedCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), graphHead: headBefore,
  journey: "scripts/e2e-block-last-changed.mjs", binary: app, tooltips: [] };
async function shot(name) {
  execFileSync("import", ["-window", "root", path.join(run, name)], { env: process.env });
}
try {
  const deadline = Date.now() + 20_000;
  let ready = false;
  while (!ready && Date.now() < deadline) {
    try { ready = (await fetch(`http://127.0.0.1:${port}/status`)).ok; } catch { /* Driver is still starting. */ }
    if (!ready) await poll(100);
  }
  assert.ok(ready, "tauri-driver did not become ready");
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    capabilities: tauriCapabilities(app, "default", process.platform, target.debuggerAddress) });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1280, 900);
  await openPageByName(browser, "History");
  await assertAppLease(account);
  const date = async stamp => browser.execute(value => new Date(value).toLocaleString(), stamp);
  async function hover(text, expected) {
    const id = await browser.execute(wanted => [...document.querySelectorAll(".ls-block")].find(row =>
      row.querySelector(":scope > .block-main .block-content")?.textContent?.includes(wanted))?.getAttribute("data-block-id"), text);
    assert.ok(id, `No rendered block: ${text}`);
    const bullet = await browser.$(`[data-block-id="${id}"] > .block-main .bullet-container`);
    await bullet.moveTo();
    const tooltip = await bullet.$('[role="tooltip"]');
    await tooltip.waitForDisplayed({ timeout: 15_000 });
    await browser.waitUntil(async () => (await tooltip.getText())?.includes(expected), {
      timeout: 15_000, interval: 100, timeoutMsg: `Bullet never showed ${expected}`,
    });
    const title = await tooltip.getText();
    assert.ok(title.includes("drag to move"), "existing gesture help disappeared");
    evidence.tooltips.push({ text, title });
    return bullet;
  }
  await hover("Edited in the second commit", `Last changed ${await date("2024-02-11T11:00:00Z")}`);
  await shot("committed-tooltip.png");
  await hover("Untouched since creation", `Last changed ${await date("2024-01-10T10:00:00Z")}`);
  await hover("Changed locally without a commit", "Changed, not yet committed");
  await shot("uncommitted-tooltip.png");
  // Returning to the same page/HEAD reuses attribution for every block.
  await hover("Edited in the second commit", `Last changed ${await date("2024-02-11T11:00:00Z")}`);
  assert.equal(fs.readFileSync(page, "utf8"), before, "hover changed the graph file");
  assert.equal(git("rev-parse", "HEAD"), headBefore, "hover changed git history");
  evidence.outcome = "PASS: commit 2 date, creation commit for untouched block, uncommitted wording, repeated hover, no graph writes";
  console.log(evidence.outcome);
} catch (error) {
  evidence.expected = "A bullet's tooltip shows its body-changing commit's local date or honest uncommitted state";
  evidence.observed = String(error);
  evidence.classification = "ambiguous";
  if (browser) await shot("failure.png");
  throw error;
} finally {
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify(evidence, null, 2) + "\n");
  if (browser) await browser.deleteSession();
  try { process.kill(-driver.pid, "SIGTERM"); } catch { /* The journey's driver has already exited. */ }
  stopWebdriverApplication(target);
  fs.closeSync(log);
  console.log(`Evidence: ${run}`);
}

// GH #346: real asset transport, image decode, watcher replacement and no remote fetch.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
import os from "node:os";
import { createHash } from "node:crypto";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = path.join(root, "artifacts/page-image-icons", `native-${Date.now()}`);
const graph = path.join(out, "graph");
for (const dir of ["assets", "pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, "xdg", dir), { recursive: true });
const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aFOkAAAAASUVORK5CYII=", "base64");
let requests = 0;
const server = http.createServer((_req, res) => { requests++; res.writeHead(200, { "Content-Type": "image/png" }); res.end(png); });
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const remoteUrl = `http://127.0.0.1:${server.address().port}/icon.png`;
fs.writeFileSync(path.join(graph, "assets/actual.png"), png);
fs.symlinkSync("actual.png", path.join(graph, "assets/logo.png"));
fs.writeFileSync(path.join(graph, "assets/next.png"), png);
const file = path.join(graph, "pages/Image.md");
// D-20261018-IC1: tine.icon:: is the documented property; icon:: (what OG prints) is the fallback.
fs.writeFileSync(file, 'tine.icon:: ![](../assets/logo.png){:width "90%"}\n\n- Image body\n');
fs.writeFileSync(path.join(graph, "pages/Remote.md"), `tine.icon:: ![](${remoteUrl})\n\n- Remote body\n`);
fs.writeFileSync(path.join(graph, "pages/Org.org"), "#+tine.icon: [[file:../assets/logo.png]]\n\n* Org body\n");
fs.writeFileSync(path.join(graph, "pages/Legacy.md"), "icon:: ![](../assets/logo.png)\n\n- Legacy body\n");
// Both properties: tine.icon wins, so the remote icon:: is neither shown nor requested.
fs.writeFileSync(path.join(graph, "pages/Both.md"), `icon:: ![](${remoteUrl})\ntine.icon:: ![](../assets/logo.png)\n\n- Both body\n`);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:favorites ["Image" "Remote" "Org" "Legacy" "Both"]}\n');
const memoryOnly = process.env.ICON_MEMORY === "1";
const memoryBytes = Number(process.env.ICON_MEMORY_BYTES || 2 * 1024 * 1024);
if (memoryOnly) {
  const names = Array.from({ length: 50 }, (_, i) => `Memory${i}`);
  for (const name of names) {
    const bytes = Buffer.alloc(memoryBytes);
    png.copy(bytes);
    fs.writeFileSync(path.join(graph, `assets/${name}.png`), bytes);
    fs.writeFileSync(path.join(graph, `pages/${name}.md`), `icon:: ![](../assets/${name}.png)\n\n- Memory fixture\n`);
  }
  fs.writeFileSync(path.join(graph, "logseq/config.edn"), `{:favorites [${names.map(name => JSON.stringify(name)).join(" ") }]}\n`);
}
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "xdg/data"), XDG_CONFIG_HOME: path.join(out, "xdg/config"), XDG_CACHE_HOME: path.join(out, "xdg/cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4858);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let journey = "open local image page";
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(), delta: "round 5 working diff", appSha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"), steps: [] };
const selectors = [".page-title-icon img[data-page-image-icon]", "#sidebar-favorites-list img[data-page-image-icon]"];
async function rendered(name = "Image") {
  for (const selector of selectors) await browser.waitUntil(() => browser.execute((selector, name) => {
    const img = selector.startsWith("#sidebar")
      ? [...document.querySelectorAll("#sidebar-favorites-list .nav-page-label")]
        .find(label => label.textContent.trim() === name)?.querySelector("img[data-page-image-icon]")
      : document.querySelector(selector);
    return !!img && img.complete && img.naturalWidth > 0;
  }, selector, name), { timeout: 15000, timeoutMsg: `${journey}: ${name}'s image did not decode in ${selector}` });
}
try {
  const deadline = Date.now() + 10000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error("driver did not become ready");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    connectionRetryTimeout: 30000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/debug/tine") } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1400, 1000);
  await browser.$(".page-title").waitForExist({ timeout: 30000 });
  if (memoryOnly) {
    journey = "50 distinct image icon pages mounted in Favorites (loaded shared machine)";
    await browser.waitUntil(() => browser.execute(() => document.querySelectorAll("#sidebar-favorites-list .nav-page-label").length === 50), { timeout: 30000 });
    // Allow the bounded read queue and browser decode to settle before sampling.
    await new Promise(resolve => setTimeout(resolve, 5000));
    const dom = await browser.execute(() => ({ favorites: document.querySelectorAll("#sidebar-favorites-list .nav-page-label").length,
      images: document.querySelectorAll("#sidebar-favorites-list img[data-page-image-icon]").length,
      decoded: [...document.querySelectorAll("#sidebar-favorites-list img[data-page-image-icon]")].filter(img => img.complete && img.naturalWidth > 0).length }));
    const processes = execFileSync("ps", ["-eo", "pid=,ppid=,comm=,rss="], { encoding: "utf8" }).trim().split("\n").map(line => {
      const [pid, parent, name, rssKiB] = line.trim().split(/\s+/); return { pid: Number(pid), parent: Number(parent), name, rssKiB: Number(rssKiB) };
    });
    const descendants = new Set([driver.pid]);
    for (let i = 0; i < processes.length; i++) for (const process of processes) if (descendants.has(process.parent)) descendants.add(process.pid);
    const renderers = processes.filter(process => descendants.has(process.pid) && process.name === "WebKitWebProces");
    if (!renderers.length || dom.favorites !== 50) throw new Error("memory fixture or renderer discovery failed");
    proof.memory = { label: "loaded shared machine; Linux WebKitGTK renderer RSS; one snapshot, no speed claim", iconFiles: 50, bytesPerFile: memoryBytes, ...dom,
      loadAverage: os.loadavg(), availableHostBytes: os.freemem(), renderers, rendererRssKiB: renderers.reduce((sum, process) => sum + process.rssKiB, 0) };
    proof.steps.push(journey);
  } else {
  await openPageByName(browser, "Image");
  await rendered();
  proof.steps.push(journey);
  journey = "replace symlink target asset while the page stays open";
  const before = await browser.$(selectors[0]).getAttribute("src");
  fs.writeFileSync(path.join(graph, "assets/actual.png"), Buffer.concat([png, Buffer.from([0])]));
  await browser.waitUntil(async () => (await browser.$(selectors[0]).getAttribute("src")) !== before, { timeout: 15000 });
  await rendered();
  proof.steps.push(journey);
  journey = "edit image property through existing icon menu and guarded save";
  await browser.execute(() => document.querySelector(".page-title").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 150 })));
  await browser.$('[data-page-action-id="set-page-icon"]').click();
  await browser.$('[data-page-icon-input]').setValue("![](../assets/next.png)");
  await browser.keys("Enter");
  await browser.waitUntil(() => fs.readFileSync(file, "utf8").includes("tine.icon:: ![](../assets/next.png)"), { timeout: 10000 });
  await rendered();
  proof.steps.push(journey);
  journey = "Org file image link renders";
  await openPageByName(browser, "Org");
  await rendered("Org");
  proof.steps.push(journey);
  journey = "icon:: fallback image renders, and tine.icon:: wins over a remote icon:: on the same page";
  await openPageByName(browser, "Legacy");
  await rendered("Legacy");
  await openPageByName(browser, "Both");
  await rendered("Both");
  proof.steps.push(journey);
  journey = "remote icon shows no image or raw-text icon in title and Favorites, preserves its property and makes no requests";
  await openPageByName(browser, "Remote");
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".page-title-icon img[data-page-image-icon]")), { timeout: 10000 });
  const remoteImages = await browser.execute(url => [...document.images].filter(img => img.src === url).length, remoteUrl);
  const refused = await browser.execute(url => ({
    titleIcon: document.querySelector(".page-title-icon")?.textContent?.trim() ?? "",
    favorite: [...document.querySelectorAll("#sidebar-favorites-list .nav-page-label")].find(label => label.textContent.includes("Remote"))?.textContent.trim(),
    propertyVisible: [...document.querySelectorAll(".page-properties .prop-value")].some(value => value.textContent.includes(url)),
  }), remoteUrl);
  if (refused.titleIcon || refused.favorite !== "Remote" || !refused.propertyVisible) throw new Error(`refused icon: ${JSON.stringify(refused)}`);
  if (remoteImages || requests) throw new Error(`remote icon: ${remoteImages} images, ${requests} network requests`);
  proof.steps.push({ journey, remoteImages, requests, ...refused });
  // The assertions above are the contract; a successful-run screenshot is
  // supplementary evidence and WebKit can refuse it independently of the DOM.
  try { await browser.saveScreenshot(path.join(out, "page-image-icons.png")); } catch (error) {
    proof.artifactFailure = { expected: "supplementary successful-run screenshot", observed: String(error), classification: "infrastructure" };
  }
  }
} catch (error) {
  proof.failure = { journey, expected: "local icons decode in title and Favorites, refresh live, and remote icons make no requests", observed: String(error), classification: "ambiguous" };
  try { proof.failure.body = await browser?.execute(() => document.body.innerText.slice(0, 2400)); await browser?.saveScreenshot(path.join(out, "failure.png")); } catch {}
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  server.close();
  console.log(`Artifacts: ${out}`);
}

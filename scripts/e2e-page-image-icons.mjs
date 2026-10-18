// GH #346: real asset transport, image decode, watcher replacement and no remote fetch.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
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
fs.writeFileSync(path.join(graph, "assets/logo.png"), png);
fs.writeFileSync(path.join(graph, "assets/next.png"), png);
const file = path.join(graph, "pages/Image.md");
fs.writeFileSync(file, 'icon:: ![](../assets/logo.png){:width "90%"}\n\n- Image body\n');
fs.writeFileSync(path.join(graph, "pages/Remote.md"), `icon:: ![](${remoteUrl})\n\n- Remote body\n`);
fs.writeFileSync(path.join(graph, "pages/Org.org"), "#+icon: [[file:../assets/logo.png]]\n\n* Org body\n");
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:favorites ["Image" "Remote" "Org"]}\n');
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "xdg/data"), XDG_CONFIG_HOME: path.join(out, "xdg/config"), XDG_CACHE_HOME: path.join(out, "xdg/cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4858);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let journey = "open local image page";
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(), delta: "round 3 working diff", steps: [] };
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
  await openPageByName(browser, "Image");
  await rendered();
  proof.steps.push(journey);
  journey = "replace asset while the page stays open";
  const before = await browser.$(selectors[0]).getAttribute("src");
  fs.writeFileSync(path.join(graph, "assets/logo.png"), Buffer.concat([png, Buffer.from([0])]));
  await browser.waitUntil(async () => (await browser.$(selectors[0]).getAttribute("src")) !== before, { timeout: 15000 });
  await rendered();
  proof.steps.push(journey);
  journey = "edit image property through existing icon menu and guarded save";
  await browser.execute(() => document.querySelector(".page-title").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 150 })));
  await browser.$('[data-page-action-id="set-page-icon"]').click();
  await browser.$('[data-page-icon-input]').setValue("![](../assets/next.png)");
  await browser.keys("Enter");
  await browser.waitUntil(() => fs.readFileSync(file, "utf8").includes("icon:: ![](../assets/next.png)"), { timeout: 10000 });
  await rendered();
  proof.steps.push(journey);
  journey = "Org file image link renders";
  await openPageByName(browser, "Org");
  await rendered("Org");
  proof.steps.push(journey);
  journey = "remote icon never renders or requests a remote image, including its visible property";
  await openPageByName(browser, "Remote");
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".page-title-icon img[data-page-image-icon]")), { timeout: 10000 });
  const remoteImages = await browser.execute(url => [...document.images].filter(img => img.src === url).length, remoteUrl);
  if (remoteImages || requests) throw new Error(`remote icon: ${remoteImages} images, ${requests} network requests`);
  proof.steps.push({ journey, remoteImages, requests });
  // The assertions above are the contract; a successful-run screenshot is
  // supplementary evidence and WebKit can refuse it independently of the DOM.
  try { await browser.saveScreenshot(path.join(out, "page-image-icons.png")); } catch (error) {
    proof.artifactFailure = { expected: "supplementary successful-run screenshot", observed: String(error), classification: "infrastructure" };
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

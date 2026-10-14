// Native semantic proof for the sweep lane. All fixtures and logs stay in its artifacts directory.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { answerNativeDialog } from "./lib/e2e-native-dialog.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = path.join(root, "artifacts/sweep-bugs", `native-${Date.now()}`);
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq", "xdg/data", "xdg/config", "xdg/cache"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "pages/Duplicate.md"), "- Original\n  - Child\n- Neighbor\n");
fs.writeFileSync(path.join(graph, "pages/Icon.md"), "icon:: 🚩\n\n- Icon body\n");
fs.writeFileSync(path.join(graph, "pages/Frame.md"), '- <iframe src="https://example.com" width="100%" height="833"></iframe>\n');
fs.writeFileSync(path.join(graph, "pages/Refusal.md"), "- Preserve this page\n");
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:shortcuts {:editor/duplicate-block-above "alt+shift+u"}}\n');
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(graph, "xdg/data"), XDG_CONFIG_HOME: path.join(graph, "xdg/config"), XDG_CACHE_HOME: path.join(graph, "xdg/cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4792);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let journey = "startup";
const proof = { commit: "bef8431713b845ad4aea1954b4be0feef4352f82 + sweep-bugs diff", steps: [] };
const check = (value, message) => { if (!value) throw new Error(message); };
try {
  const deadline = Date.now() + 10000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error("driver did not become ready");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/debug/tine") } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1200, 900);
  await browser.$(".page-title").waitForExist({ timeout: 30000 });

  journey = "GH #379 duplicate above saves a subtree";
  console.log(journey);
  await openPageByName(browser, "Duplicate");
  await browser.$(".block-content").click();
  await browser.$("textarea.block-editor").waitForExist();
  await browser.waitUntil(() => browser.execute(() => document.querySelector("textarea.block-editor")?.value === "Original"), { timeout: 5000 });
  console.log("Before duplicate", await browser.execute(() => ({ value: document.querySelector("textarea.block-editor")?.value,
    rows: [...document.querySelectorAll("[data-block-id]")].map(el => ({ id: el.dataset.blockId, text: el.innerText })) })));
  await browser.keys(["Alt", "Shift", "u"]);
  console.log("Duplicate action state", await browser.execute(() => ({
    texts: [...document.querySelectorAll(".block-content")].map(el => el.textContent),
    editor: document.querySelector("textarea.block-editor")?.value,
    toasts: [...document.querySelectorAll(".toast-msg")].map(el => el.textContent),
    banners: [...document.querySelectorAll(".conflict-bar, .page-readonly-banner")].map(el => el.textContent),
  })));
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".block-content").length >= 4), { timeout: 10000 });
  await browser.keys("Escape");
  const duplicated = path.join(graph, "pages/Duplicate.md");
  await browser.waitUntil(() => fs.readFileSync(duplicated, "utf8").split("Original").length === 3, { timeout: 10000 });
  check(fs.readFileSync(duplicated, "utf8").split("Child").length === 3, "copy lost its child on disk");
  proof.steps.push(journey);

  journey = "GH #346 icon property visible beside title icon";
  console.log(journey);
  await openPageByName(browser, "Icon");
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".page-properties")?.textContent.includes("icon")), { timeout: 10000 });
  await browser.$(".page-properties .prop-row").click();
  await browser.waitUntil(() => browser.execute(() => document.querySelector("textarea.block-editor")?.value.includes("icon:: 🚩")), { timeout: 5000 });
  await browser.keys("Escape");
  proof.steps.push(journey);

  journey = "GH #54 author iframe height and full width";
  console.log(journey);
  await openPageByName(browser, "Frame");
  await browser.$(".embed-iframe-wrap").waitForExist();
  const size = await browser.execute(() => {
    const wrap = document.querySelector(".embed-iframe-wrap");
    const frame = wrap.querySelector("iframe");
    return { height: frame.getBoundingClientRect().height, width: frame.getBoundingClientRect().width,
      containerWidth: wrap.parentElement.getBoundingClientRect().width };
  });
  check(Math.abs(size.height - 833) < 1, `author height not honored: ${JSON.stringify(size)}`);
  check(Math.abs(size.width - size.containerWidth) < 2, `author full width is capped: ${JSON.stringify(size)}`);
  proof.steps.push({ journey, size });

  journey = "GH #687 real storage refusal carries a reason and keeps the file";
  console.log(journey);
  await openPageByName(browser, "Refusal");
  // Deliberate isolated fixture failure: the trash parent is a file, so the guarded move cannot proceed.
  const trash = path.join(graph, "logseq/.tine-trash");
  check(!fs.existsSync(trash), "fixture unexpectedly already owns a trash directory");
  fs.writeFileSync(trash, "fixture storage obstruction\n");
  await browser.execute(() => document.querySelector(".page-title").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 200 })));
  await browser.$('[data-page-action-id="delete-page"]').waitForExist();
  await browser.execute(() => document.querySelector('[data-page-action-id="delete-page"]').click());
  await answerNativeDialog("yes", { env });
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".toast-msg")].some(el => el.textContent.startsWith("Delete failed:"))), { timeout: 15000 });
  const reason = await browser.execute(() => [...document.querySelectorAll(".toast-msg")].map(el => el.textContent).find(text => text.startsWith("Delete failed:")));
  check(reason.length > "Delete failed:".length + 5, "generic delete failure survived");
  check(fs.readFileSync(path.join(graph, "pages/Refusal.md"), "utf8") === "- Preserve this page\n", "failed delete changed the page");
  proof.steps.push({ journey, reason });
  console.log(JSON.stringify(proof));
} catch (error) {
  proof.failure = { journey, expected: "semantic journey completes with durable content preserved", observed: String(error), classification: "ambiguous" };
  try {
    proof.failure.ui = await browser?.execute(() => ({ titles: [...document.querySelectorAll(".page-title")].map(el => el.textContent),
      body: document.body.innerText.slice(0, 2500), messages: [...document.querySelectorAll(".toast-msg")].map(el => el.textContent) }));
  } catch {}
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  console.log(`Artifacts: ${out}`);
}

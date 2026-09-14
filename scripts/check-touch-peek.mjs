import { createServer } from "vite";
import { chromium } from "playwright";
import { mkdir, writeFile } from "node:fs/promises";
import { realpathSync } from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = process.env.TINE_PROBE_OUT ?? path.join(root, "screenshots/touch-peek");
await mkdir(out, { recursive: true });
const baseline = process.env.TINE_PEEK_BASELINE_REF;
const plugins = baseline ? [{ name: "reviewed-before-source", enforce: "pre", load(id) {
  const relative = path.relative(root, id.split("?")[0]);
  return ["src/render/inline.tsx", "src/render/longPress.ts"].includes(relative)
    ? execFileSync("git", ["show", `${baseline}:${relative}`], { cwd: root, encoding: "utf8" }) : null;
} }] : [];
const server = await createServer({ root, plugins, server: { host: "127.0.0.1", port: 0,
  fs: { allow: [root, realpathSync(path.join(root, "node_modules/inter-ui"))] } } });
await server.listen();
const browser = await chromium.launch({ args: ["--no-sandbox"] });
const results = [], failures = [];
try {
  const page = await browser.newPage({ viewport: { width: 390, height: 720 }, hasTouch: true });
  page.on("pageerror", error => console.error(error.message));
  await page.goto(`${server.resolvedUrls.local[0]}scripts/fixtures/touch-peek/index.html`);
  await page.waitForFunction(() => window.peekReady);
  await page.evaluate(() => { window.inputTrace = []; for (const type of ["pointerenter", "mouseenter", "pointerdown", "pointerup", "contextmenu", "click"]) {
    document.addEventListener(type, event => window.inputTrace.push({ type, pointerType: event.pointerType,
      trusted: event.isTrusted, target: event.target.className }), true);
  } });
  const link = page.locator("main a.page-ref");
  const box = await link.boundingBox();
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: box.x + box.width / 2, y: box.y + box.height / 2 }] });
  await page.waitForTimeout(700);
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await page.waitForTimeout(600);
  const touch = await page.evaluate(() => ({ menus: document.querySelectorAll('[aria-label="Page actions"]').length,
    previews: document.querySelectorAll('.peek-popup').length, trace: window.inputTrace }));
  results.push({ kind: "real-touch-hold", ...touch });
  if (touch.menus !== 1 || touch.previews !== 0) failures.push("touch hold must retain exactly one menu with no hover preview");
  await page.screenshot({ path: path.join(out, "touch.png") });
  // Dismiss through the actual overlay, then use a real mouse on the same hybrid page.
  await page.mouse.click(370, 680);
  await page.mouse.move(380, 650);
  await link.hover();
  await page.waitForTimeout(600);
  const mouse = await page.locator('.peek-popup').count();
  results.push({ kind: "hybrid-mouse-hover", previews: mouse });
  if (mouse !== 1) failures.push("real mouse hover must still open one preview");
  await page.mouse.move(380, 650);
  await page.waitForTimeout(200);
  const left = await page.locator('.peek-popup').count();
  results.push({ kind: "mouse-leave", previews: left });
  if (left !== 0) failures.push("mouse leave must dismiss preview");
  await writeFile(path.join(out, "results.json"), JSON.stringify({ results, failures }, null, 2));
  console.log(JSON.stringify({ failures }));
  if (failures.length) process.exitCode = 1;
} finally { await browser.close(); await server.close(); }

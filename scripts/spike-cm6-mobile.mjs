// Chromium touch/phone probe, not Android WebView or physical IME evidence.
import { chromium } from "playwright";
import { createServer } from "vite";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const out = path.resolve("artifacts/spike-cm6-stage1", process.env.SPIKE_MOBILE_RUN || "mobile");
fs.mkdirSync(out, { recursive: true });
const report = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), platform: "Chromium mobile emulation (412x900, touch, coarse pointer)", cases: [], errors: [] };
const result = (scenario, pass, observed) => { report.cases.push({ scenario, pass, observed }); console.log(JSON.stringify(report.cases.at(-1))); };
process.env.VITE_CM6_SPIKE = "1";
const server = await createServer({ cacheDir: path.join(out, "vite-cache"), server: { port: 0, fs: { allow: [process.cwd(), fs.realpathSync("node_modules/inter-ui")] } } });
let browser;
try {
  await server.listen();
  const url = server.resolvedUrls.local[0];
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  const ctx = await browser.newContext({ viewport: { width: 412, height: 900 }, hasTouch: true, isMobile: true,
    userAgent: "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36" });
  const page = await ctx.newPage();
  page.on("pageerror", e => report.errors.push(e.message));
  // No query/session activation: the build environment alone must enable CM6.
  await page.goto(`${url}scripts/fixtures/mobile-rendered-selection.html`);
  await page.locator('[data-block-id="b0"]').waitFor();
  await page.evaluate(() => window.__tine.startEditing("b0", 0, null));
  await page.locator(".cm-content").waitFor();
  await page.locator(".cm-content").press("End");
  await page.keyboard.type(" phone");
  result("default-mount-and-input", await page.evaluate(() => window.__tine.raw("b0") === "alpha selectable words in block 0 phone"));
  await page.setViewportSize({ width: 412, height: 520 });
  await page.evaluate(() => window.dispatchEvent(new CustomEvent("tine-ime", { detail: true })));
  await sleep(350);
  const dock = page.locator("[data-mobile-keyboard-toolbar]:visible");
  result("keyboard-dock-visible", await dock.count() > 0, await page.evaluate(() => ({ focused: document.activeElement?.className, editing: window.__tine.editingId() })));
  if (await dock.count()) {
    await dock.getByRole("button", { name: "Soft newline", exact: true }).tap();
    await sleep(150);
    result("keyboard-soft-newline", await page.evaluate(() => window.__tine.raw("b0").endsWith(" phone\n")));
  } else result("keyboard-soft-newline", false, "Dock unavailable for CM6");
  // Browser-native selection via keyboard, not a CM dispatch.
  await page.locator(".cm-content").press("Control+Home");
  for (let i = 0; i < 5; i++) await page.keyboard.press("Shift+ArrowRight");
  const selection = await page.evaluate(() => getSelection().toString());
  result("native-selection", selection === "alpha", { selection });
  const bold = page.locator('[data-mobile-selection-toolbar] [data-selection-action="bold"]');
  result("selection-format-toolbar-visible", await bold.isVisible());
  if (await bold.isVisible()) {
    await bold.tap();
    result("selection-bold-applies", await page.evaluate(() => window.__tine.raw("b0").startsWith("**alpha**")));
    // Select a fresh word: nested-marker toggle policy is a separate helper
    // contract, not a useful phone-toolbar oracle.
    await page.locator(".cm-content").press("Control+Home");
    for (let i = 0; i < 10; i++) await page.keyboard.press("ArrowRight");
    for (let i = 0; i < 10; i++) await page.keyboard.press("Shift+ArrowRight");
    await page.locator('[data-mobile-selection-toolbar] [data-selection-action="italic"]').tap();
    result("selection-italic-applies", await page.evaluate(() => window.__tine.raw("b0").startsWith("**alpha** *selectable*")));
  } else result("selection-bold-applies", false, "No CM6 formatting toolbar");
  await page.screenshot({ path: path.join(out, "editing-selection.png") });
  await page.locator(".cm-content").press("Control+End");
  await dock.getByRole("button", { name: "Page reference", exact: true }).tap();
  result("keyboard-page-reference", await page.evaluate(() => window.__tine.raw("b0").endsWith("[[]]")));
  await page.locator(".cm-content").press("Control+End");
  await dock.getByRole("button", { name: "Block reference", exact: true }).tap();
  result("keyboard-block-reference", await page.evaluate(() => window.__tine.raw("b0").endsWith("[[]](())")));
  const beforeTodo = await page.evaluate(() => window.__tine.raw("b0"));
  await dock.getByRole("button", { name: "TODO", exact: true }).tap();
  report.limitations = [{ action: "TODO", observed: await page.evaluate(() => window.__tine.raw("b0")) === beforeTodo ? "No effect: command not adapted" : "Changed text" }];
  await dock.getByRole("button", { name: "Hide keyboard", exact: true }).tap();
  result("keyboard-hide-ends-edit", await page.evaluate(() => !window.__tine.editingId() && !document.querySelector(".cm-content")));
  await page.setViewportSize({ width: 412, height: 900 });
  await page.evaluate(() => window.dispatchEvent(new CustomEvent("tine-ime", { detail: false })));
  // Existing rendered-selection toolbar remains relevant outside the editor.
  await page.evaluate(() => {
    const el = document.querySelector('[data-block-id="b3"] .block-content');
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    let t;
    while ((t = walker.nextNode()) && !t.textContent.includes("selectable")) {}
    const start = t.textContent.indexOf("selectable");
    getSelection().setBaseAndExtent(t, start, t, start + 10);
  });
  const renderedBold = page.locator('[data-rendered-selection-toolbar] [data-selection-action="bold"]');
  await renderedBold.waitFor({ state: "visible", timeout: 5000 });
  await renderedBold.tap();
  await sleep(150);
  result("rendered-selection-bold-applies", await page.evaluate(() => window.__tine.raw("b3").includes("**selectable**")));
  await page.screenshot({ path: path.join(out, "rendered-bold.png") });
} catch (error) {
  report.failureCapsule = { commit: report.commit, journey: "CM6 phone shell", expected: "editable block, keyboard dock and formatting actions", observed: String(error.stack || error), classification: "ambiguous" };
} finally {
  fs.writeFileSync(path.join(out, "report.json"), JSON.stringify(report, null, 2) + "\n");
  await browser?.close();
  await server.close();
}
if (report.failureCapsule || report.errors.length || report.cases.some(c => !c.pass)) process.exitCode = 1;

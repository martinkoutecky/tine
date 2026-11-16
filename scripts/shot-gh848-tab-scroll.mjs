// GH #848: switching tab and back keeps the reading position deep in Linked References. Chromium, the reporter's
// engine. Records the first row in view (and its offset) and the scrollbar before leaving, and again some time after
// coming back. Usage: npm run build && node scripts/shot-gh848-tab-scroll.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = Number(process.env.GH848_PORT || 5294);
const OUT = process.env.E2E_ARTIFACT_DIR || "artifacts/gh848-tab-scroll";
const LATENCY = Number(process.env.GH848_LATENCY || 1500); // the slow reference read of a big graph
const LIMIT = 30; // px a row may move: the reader must still be looking at the same thing
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore", detached: true });
const watchdog = setTimeout(() => { console.error("FAIL: timed out"); try { process.kill(-server.pid); } catch {} process.exit(1); }, 240_000);
const results = [], failures = [];
let browser;
// The reader's position in one pane: first row in view, its offset, and the scrollbar.
const reading = (pane) => {
  const s = document.querySelector(`${pane} .main-content, .main-content${pane}`), top = s.getBoundingClientRect().top;
  for (const row of s.querySelectorAll(".reference-group, .ls-block")) {
    const r = row.getBoundingClientRect();
    if (r.top >= top && r.height > 0) return { text: row.textContent.slice(0, 40), offset: Math.round(r.top - top), top: Math.round(s.scrollTop), height: s.scrollHeight, client: s.clientHeight };
  }
  return { text: "", offset: 0, top: Math.round(s.scrollTop), height: s.scrollHeight, client: s.clientHeight };
};
// A page link inside the viewport: a locator click would first scroll the page to the link it finds.
const clickInView = async (page, pane, modifier) => {
  const at = await page.evaluate((pane) => {
    const s = document.querySelector(`${pane} .main-content, .main-content${pane}`).getBoundingClientRect();
    for (const b of document.querySelectorAll(`${pane} a.page-ref, ${pane} button.reference-page`)) {
      const r = b.getBoundingClientRect();
      if (r.height > 0 && r.top > s.top + 8 && r.bottom < s.bottom - 8) return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    }
    return null;
  }, pane);
  if (!at) throw new Error("no page link in view");
  await page.keyboard.down(modifier);
  await page.mouse.click(at.x, at.y);
  await page.keyboard.up(modifier);
};
const openHub = async (page, pane) => {
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("Hub");
  await page.waitForTimeout(400);
  await page.locator(".switcher-row", { hasText: "Hub" }).first().click();
  await expandReferences(page, pane);
};
const expandReferences = async (page, pane) => {
  const refs = page.locator(`${pane} .linked-references`);
  await refs.locator(".references-header").waitFor({ timeout: 30_000 });
  if (!(await refs.locator(".reference-blocks").count())) await refs.locator(".references-header").click();
  await refs.locator(".reference-blocks").first().waitFor({ timeout: 30_000 });
};
const scrollTo = async (page, pane, fraction) => {
  for (let i = 0; i < 200; i++) {
    const done = await page.evaluate(([pane, target]) => { const s = document.querySelector(`${pane} .main-content, .main-content${pane}`); if (s.scrollTop + s.clientHeight >= s.scrollHeight * target) return 1; s.scrollTop += 500; return 0; }, [pane, fraction]);
    await page.waitForTimeout(80);
    if (done) break;
  }
  await page.waitForTimeout(1500);
};
// A pane's tab strip sits outside its content element and names the pane.
const tabs = (pane) => `[data-tab-strip-pane-id="${pane.match(/"(.*)"/)[1]}"] .tab`;
const switchTabs = async (page, pane) => {
  await page.locator(tabs(pane)).nth(1).click();
  await page.waitForTimeout(600);
  await page.locator(tabs(pane)).nth(0).click();
};
const CASES = [
  // Main pane tabs on a routed page, at several depths.
  [0.2, "tab"], [0.5, "tab"], [0.8, "tab"], [0.97, "tab"],
  [0.5, "sidebar"], // work in a right-sidebar item: the main pane is not left at all
  [0.5, "back"],    // history back to the routed page
  [0.5, "split"],   // tabs of the second pane of a split
  [0.5, "journal"], // tabs, starting from the journal feed
];
try {
  for (let i = 0; i < 80; i++) { try { if ((await fetch(`http://localhost:${PORT}/`)).ok) break; } catch {} await sleep(250); }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const [fraction, away] of CASES) {
    console.log(`case ${away}@${fraction}`);
    const page = await browser.newPage({ viewport: { width: 1400, height: 800 } });
    await page.goto(`http://localhost:${PORT}/?manyrefs&refslatency=${LATENCY}`);
    await page.waitForSelector(".ls-block", { timeout: 30_000 });
    let pane = '[data-pane-id="main"]';
    if (away === "journal") {
      await clickInView(page, pane, "Control");
    } else {
      await openHub(page, pane);
      if (away === "split") {
        await page.evaluate(() => { document.querySelector(".main-content").scrollTop = 0; });
        await page.locator(`${pane} button.reference-page`).first().click({ modifiers: ["Alt"] });
        await page.waitForFunction(() => document.querySelectorAll("[data-pane-id]").length > 1, null, { timeout: 10_000 });
        const id = await page.evaluate(() => [...document.querySelectorAll("[data-pane-id]")].map((e) => e.dataset.paneId).find((p) => p !== "main"));
        pane = `[data-pane-id="${id}"]`;
        await page.locator(`${pane} a.page-ref`, { hasText: "Hub" }).first().click();
        await expandReferences(page, pane);
      }
      await page.locator(`${pane} button.reference-page`).first().click({ modifiers: ["Control"] });
    }
    await page.waitForFunction((selector) => document.querySelectorAll(selector).length > 1, tabs(pane), { timeout: 10_000 });
    await scrollTo(page, pane, fraction);
    const before = await page.evaluate(reading, pane);
    if (away === "sidebar") {
      await clickInView(page, pane, "Shift");
      await page.waitForSelector("[data-sidebar-surface], .right-sidebar .ls-block", { timeout: 10_000 });
      await page.locator("[data-sidebar-surface], .right-sidebar .ls-block").first().click({ position: { x: 20, y: 10 } });
    } else if (away === "back") {
      await page.keyboard.press("Control+k");
      await page.locator(".switcher-input").fill("Plain neighbour");
      await page.waitForTimeout(300);
      await page.locator(".switcher-row", { hasText: "Plain neighbour" }).first().click();
      await page.waitForTimeout(600);
      await page.keyboard.press("Alt+ArrowLeft");
    } else {
      await switchTabs(page, pane);
    }
    await page.waitForTimeout(100);
    const early = await page.evaluate(reading, pane);
    await page.waitForTimeout(3000 + LATENCY);
    const after = await page.evaluate(reading, pane);
    const moved = after.text === before.text ? Math.abs(after.offset - before.offset) : Infinity;
    results.push({ fraction, away, before, early, after, moved: moved === Infinity ? "other row" : moved });
    if (moved > LIMIT) failures.push(`${away}@${fraction}: ${JSON.stringify(before)} -> ${JSON.stringify(after)}`);
    await page.close();
  }
  writeFileSync(path.join(OUT, "result.json"), JSON.stringify(results, null, 1));
  console.log(JSON.stringify(results.map((r) => ({ away: r.away, f: r.fraction, moved: r.moved, top: [r.before.top, r.early.top, r.after.top], height: [r.before.height, r.early.height, r.after.height], text: [r.before.text, r.after.text] }))));
} catch (error) {
  failures.push(String(error));
} finally {
  clearTimeout(watchdog);
  await browser?.close().catch(() => {});
  try { process.kill(-server.pid); } catch {}
}
if (failures.length) { console.error(`FAIL: ${failures.join("\n")}`); process.exit(1); }
console.log("PASS: every return kept the reading position");

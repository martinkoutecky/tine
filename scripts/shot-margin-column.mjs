// Visual check for margin dialogue slice 2 (vision §3.7 "Layout"): the comment
// column at two desktop sizes in light and dark, for the slice-1 fixture page,
// a paragraph with six comments (stacking), and a page without comments (whose
// layout must stay the plain centred column).
// Usage: npm run build && node scripts/shot-margin-column.mjs [outDir]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5218;
const OUT = process.argv[2] ?? "scratch/margin-s2";
const SIZES = [{ width: 1440, height: 900 }, { width: 1100, height: 800 }];
const MODES = ["light", "dark"];
const PAGES = [
  { slug: "comments", name: "Margin comments regression", wait: "This sentence was rewritten since.", threads: 3 },
  { slug: "stacking", name: "Margin stacking regression", wait: "Name the norm.", threads: 6 },
  { slug: "plain", name: "Tine", wait: null, threads: 0 },
];
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
let browser;
let failed = false;

async function waitForServer(url) {
  for (let i = 0; i < 60; i++) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

// Geometry read in the page: the pane, the column, each thread and the top of
// the passage (or the commented block's row) it belongs to.
function facts() {
  const pane = document.querySelector(".main-content");
  const inner = document.querySelector(".main-content-inner");
  const style = getComputedStyle(inner);
  const threads = [...document.querySelectorAll(".margin-thread")].map((el) => {
    const id = el.dataset.threadId;
    const mark = document.querySelector(`.page-blocks .comment-quote-anchor[data-comment-id="${id}"]`);
    const rect = el.getBoundingClientRect();
    return { id, top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right, anchor: mark ? mark.getBoundingClientRect().top : null };
  });
  const outline = document.querySelector(".page-blocks").getBoundingClientRect();
  return {
    paneWidth: pane.clientWidth,
    marginActive: !!document.querySelector(".page-blocks.margin-active"),
    column: !!document.querySelector(".margin-column"),
    innerPaddingRight: style.paddingRight,
    innerMaxWidth: style.maxWidth,
    outlineRight: outline.right,
    markers: [...document.querySelectorAll(".margin-marker")].map((m) => m.textContent),
    commentsInOutline: document.querySelectorAll(".page-blocks .ls-block.comment-card").length,
    threads,
    overflow: document.documentElement.scrollWidth > window.innerWidth,
  };
}

function check(name, page, f) {
  const fail = (why) => { throw new Error(`${name}: ${why}\n${JSON.stringify(f)}`); };
  if (f.overflow) fail("horizontal page overflow");
  if (page.threads === 0 || f.paneWidth < 960) {
    if (f.column || f.marginActive || f.markers.length) fail("margin drawn where it should not be");
    if (f.innerPaddingRight !== "48px") fail(`main column padding changed: ${f.innerPaddingRight}`);
    if (page.threads > 0 && f.commentsInOutline === 0) fail("narrow pane lost its inline comments");
    return;
  }
  if (!f.column || !f.marginActive) fail("no margin column on a wide pane");
  if (f.commentsInOutline !== 0) fail("comments drawn twice");
  if (f.threads.length !== page.threads) fail(`${f.threads.length} threads, expected ${page.threads}`);
  const sorted = [...f.threads].sort((a, b) => a.top - b.top);
  for (let i = 1; i < sorted.length; i++) if (sorted[i].top < sorted[i - 1].bottom) fail(`threads ${sorted[i - 1].id} and ${sorted[i].id} overlap`);
  for (const t of f.threads) {
    if (t.left < f.outlineRight) fail(`thread ${t.id} overlaps the outline`);
    if (t.anchor !== null && t.top < t.anchor - 1) fail(`thread ${t.id} sits above its passage`);
  }
  // The topmost thread has nothing above it to make room for.
  const first = sorted[0];
  if (first.anchor !== null && Math.abs(first.top - first.anchor) > 1) fail(`first thread ${first.id} is not aligned with its passage`);
}

try {
  mkdirSync(OUT, { recursive: true });
  await waitForServer(`http://localhost:${PORT}/?regressions`);
  browser = await chromium.launch();
  for (const size of SIZES) for (const mode of MODES) for (const page of PAGES) {
    const name = `${page.slug}-${size.width}-${mode}`;
    const tab = await browser.newPage({ viewport: size });
    tab.setDefaultTimeout(5000);
    const errors = [];
    tab.on("console", (message) => message.type() === "error" && errors.push(message.text()));
    tab.on("pageerror", (error) => errors.push(String(error)));
    await tab.goto(`http://localhost:${PORT}/?regressions`);
    await tab.waitForSelector(".ls-block", { timeout: 5000 });
    await tab.evaluate((m) => document.documentElement.setAttribute("data-theme", m), mode);
    await tab.keyboard.press("Control+k");
    await tab.locator(".switcher-input").fill(page.name);
    await tab.locator(".switcher-row").first().click();
    if (page.wait) await tab.getByText(page.wait).first().waitFor({ timeout: 5000 });
    else await tab.locator(".page-title").filter({ hasText: page.name }).first().waitFor({ timeout: 5000 });
    await sleep(400);
    const f = await tab.evaluate(facts);
    console.log(JSON.stringify({ case: name, paneWidth: f.paneWidth, margin: f.column, markers: f.markers, threads: f.threads.map((t) => [t.id.slice(0, 6), Math.round(t.top), t.anchor === null ? null : Math.round(t.anchor)]) }));
    check(name, page, f);
    if (errors.length) throw new Error(`${name}: browser errors:\n${errors.join("\n")}`);
    await tab.screenshot({ path: `${OUT}/${name}.png`, fullPage: false });
    if (page.slug === "stacking" && f.column) {
      // Hover the fourth thread: it and its passage are emphasised.
      await tab.locator(".margin-thread").nth(3).hover();
      const emphasised = await tab.evaluate(() => ({
        thread: document.querySelectorAll(".margin-thread.active").length,
        mark: document.querySelectorAll(".page-blocks .comment-quote-anchor.active").length,
      }));
      if (emphasised.thread !== 1 || emphasised.mark !== 1) throw new Error(`${name}: hover emphasis ${JSON.stringify(emphasised)}`);
      await tab.screenshot({ path: `${OUT}/${name}-hover.png`, fullPage: false });
    }
    await tab.close();
  }
  console.log(`screenshots in ${OUT}`);
} catch (error) {
  failed = true;
  console.error(error);
} finally {
  await browser?.close();
  server.kill();
}
process.exit(failed ? 1 : 0);

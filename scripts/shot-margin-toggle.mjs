// Visual check for GH #845: the comment margin shown, hidden with the t m
// shortcut, and shown again, at a wide pane (the margin column) and a narrow one
// (inline cards), in light and dark. Before the change the page can only be shown.
// Usage: npm run build && node scripts/shot-margin-toggle.mjs [outDir]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = Number(process.env.MARGIN_SHOT_PORT || 5219);
const OUT = process.argv[2] ?? "artifacts/margin-toggle-845/shots";
const SIZES = [{ slug: "wide", width: 1440, height: 900 }, { slug: "narrow", width: 800, height: 800 }];
const PAGE = "Margin comments regression";
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore", cwd: process.env.MARGIN_SHOT_ROOT });
let browser;

async function waitForServer(url) {
  for (let i = 0; i < 60; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

const drawn = () => ({
  column: !!document.querySelector(".margin-column"),
  threads: document.querySelectorAll(".margin-thread").length,
  cards: document.querySelectorAll(".ls-block.comment-card").length,
  markers: document.querySelectorAll(".margin-marker").length,
  marks: document.querySelectorAll(".comment-quote-anchor").length,
  overflow: document.documentElement.scrollWidth > window.innerWidth,
});
const none = (f) => !f.column && f.threads === 0 && f.cards === 0 && f.markers === 0 && f.marks === 0;

try {
  mkdirSync(OUT, { recursive: true });
  await waitForServer(`http://localhost:${PORT}/?regressions`);
  browser = await chromium.launch();
  for (const size of SIZES) for (const mode of ["light", "dark"]) {
    const name = `${size.slug}-${mode}`;
    const tab = await browser.newPage({ viewport: { width: size.width, height: size.height } });
    tab.setDefaultTimeout(5000);
    const errors = [];
    tab.on("console", (message) => message.type() === "error" && errors.push(message.text()));
    tab.on("pageerror", (error) => errors.push(String(error)));
    await tab.goto(`http://localhost:${PORT}/?regressions`);
    await tab.waitForSelector(".ls-block");
    await tab.evaluate((m) => document.documentElement.setAttribute("data-theme", m), mode);
    await tab.keyboard.press("Control+k");
    await tab.locator(".switcher-input").fill(PAGE);
    await tab.locator(".switcher-row").first().click();
    await tab.getByText("A block I wrote myself, with no comments.").first().waitFor();
    await sleep(400);
    const shown = await tab.evaluate(drawn);
    if (none(shown)) throw new Error(`${name}: nothing drawn before hiding ${JSON.stringify(shown)}`);
    await tab.screenshot({ path: `${OUT}/${name}-1-shown.png` });
    await tab.locator(".main-content").click({ position: { x: 5, y: 5 } });
    await tab.keyboard.press("t");
    await tab.keyboard.press("m");
    await sleep(400);
    const hidden = await tab.evaluate(drawn);
    if (!none(hidden) || hidden.overflow) throw new Error(`${name}: threads drawn while hidden ${JSON.stringify(hidden)}`);
    await tab.screenshot({ path: `${OUT}/${name}-2-hidden.png` });
    // Survives a reload (the device preference), then comes back.
    await tab.reload();
    await tab.waitForSelector(".ls-block");
    const reloaded = await tab.evaluate(() => localStorage.getItem("logseq-claude.comment-margin-hidden"));
    if (reloaded !== "1") throw new Error(`${name}: preference not stored (${reloaded})`);
    await tab.keyboard.press("Control+k");
    await tab.locator(".switcher-input").fill(PAGE);
    await tab.locator(".switcher-row").first().click();
    await tab.getByText("A block I wrote myself, with no comments.").first().waitFor();
    await sleep(400);
    const afterReload = await tab.evaluate(drawn);
    if (!none(afterReload)) throw new Error(`${name}: threads drawn after a reload although hidden ${JSON.stringify(afterReload)}`);
    await tab.screenshot({ path: `${OUT}/${name}-3-hidden-after-reload.png` });
    await tab.locator(".main-content").click({ position: { x: 5, y: 5 } });
    await tab.keyboard.press("t");
    await tab.keyboard.press("m");
    await sleep(600);
    const back = await tab.evaluate(drawn);
    if (none(back)) throw new Error(`${name}: threads did not come back ${JSON.stringify(back)}`);
    if (errors.length) throw new Error(`${name}: browser errors:\n${errors.join("\n")}`);
    await tab.screenshot({ path: `${OUT}/${name}-4-shown-again.png` });
    console.log(JSON.stringify({ case: name, shown, hidden, afterReload, back }));
    await tab.close();
  }
  console.log("PASS: the margin hides on t m in both layouts, stays hidden after a reload, and comes back");
} finally {
  await browser?.close();
  server.kill();
}

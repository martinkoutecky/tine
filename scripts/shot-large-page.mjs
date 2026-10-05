// Production App/router measurement. Same fixture as npm run bench; five pinned
// repeats include open, leave, reopen and history. Raw long tasks are retained.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const label = process.argv[2] ?? "after";
const port = 5271;
const output = `test-results/qf1/${label}`;
mkdirSync(output, { recursive: true });
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "preview", "--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://localhost:${port}`)).ok) break; } catch {}
    await sleep(100);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const results = [];
  for (const [count, long] of [[1, false], [60, false], [2000, false], [5000, false], [2000, true]]) {
    for (let repeat = 0; repeat < 5; repeat++) {
      const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
      await page.addInitScript(() => {
        window.__tineBench = true;
        window.__longTasks = [];
        new PerformanceObserver((list) => window.__longTasks.push(...list.getEntries().map((e) => ({ start: e.startTime, duration: e.duration })))).observe({ type: "longtask", buffered: true });
      });
      await page.goto(`http://localhost:${port}/?big&blocks=${count}${long ? "&long" : ""}`);
      await page.waitForSelector(".page-title");
      await sleep(300);
      const measure = async (action, expected) => {
        return page.evaluate(async ({ action, expected }) => {
          window.__longTasks = [];
          const start = performance.now();
          if (action === "open") document.querySelector(".switcher-row").dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
          else if (action === "leave") [...document.querySelectorAll(".nav-item")].find((e) => e.textContent.includes("Journals")).click();
          else window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", altKey: true, bubbles: true }));
          let stable = 0, previous = -1;
          await new Promise((resolve, reject) => {
            const tick = () => {
              const title = document.querySelector(".page-title")?.textContent?.trim();
              const n = document.querySelectorAll(".ls-block").length;
              if ((expected === "Big" ? title === "Big" : title !== "Big") && n && n === previous && !document.querySelector(".page-loading")) stable++;
              else stable = 0;
              previous = n;
              if (stable >= 3) resolve();
              else if (performance.now() - start > 15000) reject(new Error(`Navigation did not settle: ${action} title=${title} shells=${n}`));
              else requestAnimationFrame(tick);
            };
            requestAnimationFrame(tick);
          });
          const ms = performance.now() - start;
          await new Promise((resolve) => setTimeout(resolve, 0));
          return { ms, start, end: start + ms, shells: document.querySelectorAll(".ls-block").length, nodes: document.querySelectorAll("*").length,
            longTasks: window.__longTasks.filter((t) => t.start < start + ms && t.start + t.duration > start) };
        }, { action, expected });
      };
      const prepare = async () => {
        await page.keyboard.press("Control+k");
        await page.locator(".switcher-input").fill("Big");
        await page.waitForFunction(() => document.querySelector(".switcher-row")?.textContent.includes("Big"));
        await sleep(150);
      };
      await prepare();
      const open = await measure("open", "Big");
      await sleep(150);
      const leave = await measure("leave", "Journals");
      await prepare();
      const reopen = await measure("open", "Big");
      await sleep(150);
      await measure("leave", "Journals");
      const back = await measure("back", "Big");
      results.push({ count, long, repeat, open, leave, reopen, back });
      writeFileSync(`${output}/samples.json`, JSON.stringify(results, null, 2));
      if (repeat === 0 && count === 2000 && !long) await page.screenshot({ path: `${output}/top.png` });
      await page.close();
    }
    const rows = results.filter((r) => r.count === count && r.long === long);
    const median = (name) => rows.map((r) => r[name].ms).sort((a, b) => a - b)[2].toFixed(1);
    console.log(`${count}${long ? " long" : ""}: open ${median("open")} leave ${median("leave")} reopen ${median("reopen")} back ${median("back")} ms`);
  }
} finally {
  await browser?.close();
  server.kill("SIGKILL");
}

// Same-machine A/B of the query rendering boundary. Synthetic Rust answers
// isolate frontend work; native journeys separately prove engine semantics.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
const baseline = path.resolve(process.argv[2] ?? "artifacts/queries/baseline-source");
const resultFile = process.argv[3] ?? "artifacts/queries/query-heavy-bench.json";
const servers = [baseline, process.cwd()].map((root, index) => spawn(process.execPath, ["node_modules/vite/bin/vite.js", root, "--port", String(5356 + index), "--strictPort"], { stdio: "ignore" }));
let browser;
try {
  for (const port of [5356, 5357]) for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://localhost:${port}/scripts/harness/calm-query.html`, { signal: AbortSignal.timeout(5000) })).ok) break; } catch {}
    await sleep(200);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const samples = [[], []];
  for (let run = 0; run < 7; run++) for (const index of (run % 2 ? [1, 0] : [0, 1])) {
    const page = await browser.newPage({ viewport: { width: 1100, height: 900 } });
    page.on("pageerror", error => console.error(`case ${index}: ${error.message}`));
    page.on("console", message => { if (message.type() === "error") console.error(`case ${index}: ${message.text()}`); });
    page.on("response", response => { if (response.status() >= 400) console.error(`case ${index}: ${response.status()} ${response.url()}`); });
    await page.goto(`http://localhost:${5356 + index}/scripts/harness/calm-query.html?queries=40&rows=30`);
    await page.waitForFunction(() => document.querySelectorAll(".query-count").length >= 40 && [...document.querySelectorAll(".query-count")].every(count => count.textContent === "30")).catch(async error => {
      console.error(`case ${index}:`, await page.evaluate(() => ({ text: document.body.textContent?.slice(0, 3000), counts: [...document.querySelectorAll(".query-count")].map(count => count.textContent) })));
      throw error;
    });
    const elapsed = await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(() => resolve(performance.now() - window.__calmBenchStart))))));
    if (run) samples[index].push(elapsed);
    await page.close();
  }
  const minimum = samples.map(values => Math.min(...values));
  const delta = (minimum[1] / minimum[0] - 1) * 100;
  fs.writeFileSync(resultFile, JSON.stringify({ fixture: "40 queries × 30 result rows, List", samples, minimum, deltaPercent: delta }, null, 2));
  console.log(`Query-heavy rendering: base ${minimum[0].toFixed(1)} ms, current ${minimum[1].toFixed(1)} ms, delta ${delta.toFixed(1)}%`);
  if (delta > 0) throw new Error("Query-heavy rendering shows a regression; stop and report");
} finally { await browser?.close(); for (const server of servers) server.kill(); }

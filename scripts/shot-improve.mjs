// "Help improve Tine" diff panel — empty state + a populated report (via the
// __tineDiffFixture hook, so no live mldoc/lsdoc run is needed for the shot).
// Headless Chromium over the mock backend. → screenshots/improve-*.png
// Usage (after `source scripts/env.sh && npm run build`):
//   node scripts/shot-improve.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5219;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
const wait = async (u, t = 40) => { for (let i = 0; i < t; i++) { try { const r = await fetch(u); if (r.ok) return; } catch {} await sleep(250); } throw new Error("no server"); };

const FIXTURE = {
  tineVersion: "0.5.6-test",
  lsdocVersion: "v0.5.3",
  stats: { files: 128, totalBytes: 486213 },
  lsdocAvailable: true,
  bench: {
    lsdoc: { totalMs: 41.2, fileCount: 128, p50Ms: 0.21, p95Ms: 0.94, maxMs: 3.1, slowest: [], failures: [] },
    mldoc: { totalMs: 158.7, fileCount: 128, p50Ms: 0.83, p95Ms: 3.6, maxMs: 22.4, slowest: [], failures: [] },
  },
  findings: [
    {
      type: "divergence",
      rel: "graph-file-0001.md",
      lineStart: 14,
      lineEnd: 14,
      contextDependent: false,
      anonymized: {
        ok: true,
        tier: "tier 1",
        input: "- aaaaa **aaaa** aaa >aaaa\n  aaaaaaaa",
        lsdocKey: '{"blocks":[{"inline":[{"k":"plain","text":"..."}],"kind":"bullet","level":1}],"refs":{"block":[],"page":[]}}',
        mldocKey: '{"blocks":[{"inline":[{"k":"plain","text":"..."}],"kind":"paragraph"}],"refs":{"block":[],"page":[]}}',
      },
    },
    {
      type: "divergence",
      rel: "graph-file-0002.md",
      lineStart: 3,
      lineEnd: 5,
      contextDependent: true,
      anonymized: { ok: false },
    },
  ],
};

try {
  await wait(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(400);

  await page.locator('button.icon-btn[title^="Settings"]').first().click();
  await page.waitForSelector(".settings-modal", { timeout: 3000 });
  await page.locator(".settings-nav-item", { hasText: "Help improve Tine" }).first().click();
  await page.waitForSelector(".improve-tab", { timeout: 3000 });
  await sleep(300);
  await page.screenshot({ path: `${OUT}/improve-empty.png` });
  console.log("OK    improve-empty");

  // Inject the fixture and Run → the panel renders a populated report instantly.
  await page.evaluate((f) => { window.__tineDiffFixture = f; }, FIXTURE);
  await page.locator(".improve-run .btn-primary").click();
  await page.waitForSelector(".improve-report", { timeout: 3000 });
  await sleep(300);
  await page.screenshot({ path: `${OUT}/improve-report.png` });
  console.log("OK    improve-report");

  // Scroll the modal body to the divergence cards (the key feature is below the fold).
  await page.evaluate(() => {
    const body = document.querySelector(".settings-pane-body");
    if (body) body.scrollTop = body.scrollHeight;
  });
  await sleep(300);
  await page.screenshot({ path: `${OUT}/improve-findings.png` });
  console.log("OK    improve-findings");

  await browser.close();
  console.log("done");
} finally {
  server.kill("SIGTERM");
}

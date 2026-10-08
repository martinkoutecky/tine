// Measure horizontal alignment of the child indent guide-line vs the parent
// bullet center. OG aligns them; Tine currently doesn't. Prints, for each block
// that has rendered children, the parent-bullet center x and the guide-line
// (.block-children border-left) x, and their delta. Usage (after env.sh):
//   node scripts/measure-guides.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5199;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "inherit" });

async function waitForServer(url, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1180, height: 820 }, deviceScaleFactor: 1 });
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(500);

  const rows = await page.evaluate(() => {
    const out = [];
    const blocks = Array.from(document.querySelectorAll(".ls-block"));
    for (const b of blocks) {
      // direct-child children-container (not a deeper descendant's)
      const cc = Array.from(b.children).find((c) => c.classList.contains("block-children-container"));
      if (!cc) continue;
      const childrenEl = cc.querySelector(":scope > .block-children");
      const bullet = b.querySelector(":scope > .block-main .bullet");
      if (!childrenEl || !bullet) continue;
      const br = bullet.getBoundingClientRect();
      const cr = childrenEl.getBoundingClientRect();
      const bulletCenter = br.left + br.width / 2;
      const lineX = cr.left; // border-left sits at the left edge of the border-box
      out.push({
        text: (b.querySelector(":scope > .block-main .block-content-wrapper")?.textContent || "").slice(0, 28),
        bulletCenter: Math.round(bulletCenter * 10) / 10,
        lineX: Math.round(cr.left * 10) / 10,
        delta: Math.round((lineX - bulletCenter) * 10) / 10,
      });
    }
    return out.slice(0, 12);
  });

  console.log("text                          bulletCenter  lineX   delta(line-bullet)");
  for (const r of rows) {
    console.log(
      r.text.padEnd(30),
      String(r.bulletCenter).padStart(8),
      String(r.lineX).padStart(8),
      String(r.delta).padStart(8),
    );
  }
  await browser.close();
} finally {
  server.kill("SIGTERM");
}

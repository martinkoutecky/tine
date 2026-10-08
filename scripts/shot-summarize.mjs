// Verify + screenshot the query-builder "+ summarize" control (result
// aggregation + group-by). Headless Chromium over the mock backend — the top
// journal has a `{{query (todo TODO DOING)}}` block whose builder bar now shows a
// "+ summarize" pill. We open the popover, apply Count, then Group by page, then
// Sum of a (non-numeric) property to confirm the skip surfacing.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5259;
const OUT = "screenshots";
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

const shot = async (page, name, loc) => {
  await loc.scrollIntoViewIfNeeded();
  await sleep(150);
  const box = await loc.boundingBox();
  if (!box) { console.log(`no box for ${name}`); return; }
  await page.screenshot({
    path: `${OUT}/${name}.png`,
    clip: { x: Math.max(0, box.x - 8), y: Math.max(0, box.y - 8), width: Math.min(1200, box.width + 16), height: Math.min(1290 - Math.max(0, box.y - 8), box.height + 16) },
  });
  console.log(`wrote ${OUT}/${name}.png`);
};

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1200, height: 1300 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(500);

  // The first builder bar's summarize pill (label flips "+ summarize" → "∑ …"
  // once active, so match either). Only this query gets a summary, so read the
  // summary elements globally; screenshot the query block that owns the summary.
  const pill = () => page.locator(".qb-sort").filter({ hasText: /summarize|∑/ }).first();
  const summaryBlock = () =>
    page.locator(".query-block").filter({ has: page.locator(".query-summary, .query-summary-table") }).first();

  // Open the summarize popover.
  await pill().scrollIntoViewIfNeeded();
  await pill().scrollIntoViewIfNeeded();
  await pill().click();
  await page.waitForSelector(".qb-picker", { timeout: 6000 });
  await sleep(200);
  const titles = await page.locator(".qb-picker .qb-picker-title").allInnerTexts();
  console.log("popover sections:", titles.join(" | "));
  await shot(page, "summarize-popover", page.locator(".qb-picker").first());

  // Each pick applies and closes the popover. Apply Count → single summary line.
  await page.locator(".qb-menu-item", { hasText: "Count" }).first().click();
  await sleep(400);
  const line = await page.locator(".query-summary").first().innerText().catch(() => "(none)");
  console.log("count summary:", line.replace(/\n/g, " "));
  await shot(page, "summarize-count", summaryBlock());

  // Re-open, add Group by page → grouped count-per-page table (should total 5).
  await pill().scrollIntoViewIfNeeded();
  await pill().click();
  await page.waitForSelector(".qb-picker", { timeout: 6000 });
  await sleep(150);
  await page.locator(".qb-menu-item", { hasText: "Page" }).first().click();
  await sleep(400);
  const rows = await page.locator(".query-summary-table tbody tr").allInnerTexts().catch(() => []);
  const total = rows.reduce((a, r) => a + (parseInt(r.split(/\s+/).pop(), 10) || 0), 0);
  console.log(`grouped rows (${rows.length}), total=${total}:`, rows.map((r) => r.replace(/\s+/g, "=")).join(" · "));
  await shot(page, "summarize-grouped", summaryBlock());

  // Switch to Sum of a (non-numeric) property → confirm the skip surfacing renders.
  await pill().scrollIntoViewIfNeeded();
  await pill().click();
  await page.waitForSelector(".qb-picker", { timeout: 6000 });
  await sleep(150);
  await page.locator(".qb-menu-item", { hasText: "Sum of a property" }).first().click();
  await sleep(200);
  await page.locator(".qb-picker .qb-menu-item").first().click(); // first facet property
  await sleep(400);
  const sumLine = await page.locator(".query-summary-table, .query-summary").first().innerText().catch(() => "(none)");
  console.log("sum summary:", sumLine.replace(/\n/g, " "));

  await browser.close();
  console.log("DONE");
} catch (e) {
  console.log("ERROR:", String(e).split("\n").slice(0, 3).join(" | "));
  process.exitCode = 2;
} finally {
  server.kill("SIGTERM");
}

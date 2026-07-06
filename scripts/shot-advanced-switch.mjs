// Verify + screenshot the query-builder "⚙ advanced" switch (1b): clicking it
// replaces the visual builder with a Datalog `[:find …]` skeleton, the chip bar
// disappears (datalog isn't builder-representable), the "Partial datalog — ran: …"
// note shows, and results still render. Headless Chromium over the mock backend.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5263;
const OUT = "screenshots";
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

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
  const page = await browser.newPage({ viewport: { width: 1200, height: 1300 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(500);

  const adv = page.locator(".qb-advanced").first();
  const builderBarsBefore = await page.locator(".qb-bar").count();
  console.log("qb-bars before switch:", builderBarsBefore);
  await adv.scrollIntoViewIfNeeded();
  await adv.click();
  await sleep(700);

  const builderBarsAfter = await page.locator(".qb-bar").count();
  const note = await page.locator(".query-adv-note").first().innerText().catch(() => "(no note)");
  console.log("qb-bars after switch:", builderBarsAfter, "(one fewer means the switched query hid its builder)");
  console.log("adv note:", note.replace(/\n/g, " "));
  // The switched query should now show datalog results (ran: task).
  const count = await page.locator(".query-count").first().innerText().catch(() => "?");
  console.log("result count on switched query:", count);

  const qblk = page.locator(".query-block").filter({ has: page.locator(".query-adv-note") }).first();
  await qblk.scrollIntoViewIfNeeded();
  await sleep(200);
  const box = await qblk.boundingBox();
  if (box) {
    await page.screenshot({
      path: `${OUT}/advanced-switch.png`,
      clip: { x: Math.max(0, box.x - 8), y: Math.max(0, box.y - 8), width: Math.min(1200, box.width + 16), height: Math.min(1290 - Math.max(0, box.y - 8), box.height + 16) },
    });
    console.log(`wrote ${OUT}/advanced-switch.png`);
  }

  await browser.close();
  console.log("DONE");
} catch (e) {
  console.log("ERROR:", String(e).split("\n").slice(0, 3).join(" | "));
  process.exitCode = 2;
} finally {
  server.kill("SIGTERM");
}

// Capture the PDF reader at phone width. Run against a baseline URL as well as
// the current preview to retain visual before/after evidence:
//
//   TINE_PDF_MOBILE_BASELINE_URL=http://localhost:5226 \
//   TINE_PDF_MOBILE_AFTER_URL=http://localhost:5227 \
//   node scripts/shot-pdf-mobile.mjs
//
// With no AFTER URL, the script starts `vite preview` on 5227. The baseline is
// optional for ad-hoc after-only checks, but release evidence must supply it.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5227;
const OUT = "screenshots";
const PHONE = { width: 390, height: 844 };
const ANDROID_UA =
  "Mozilla/5.0 (Linux; Android 15; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125 Mobile Safari/537.36";
const baselineUrl = process.env.TINE_PDF_MOBILE_BASELINE_URL;
const afterUrl = process.env.TINE_PDF_MOBILE_AFTER_URL ?? `http://localhost:${PORT}`;
let server;

mkdirSync(OUT, { recursive: true });

async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      // Preview is still starting.
    }
    await sleep(250);
  }
  throw new Error(`server did not start: ${url}`);
}

async function openPdfAtPhoneWidth(browser, url, outputPath) {
  const context = await browser.newContext({
    viewport: PHONE,
    deviceScaleFactor: 2,
    isMobile: true,
    userAgent: ANDROID_UA,
  });
  const page = await context.newPage();
  try {
    await page.goto(url);
    await page.waitForSelector(".page-title", { timeout: 8000 });
    // The mobile layout opens the sidebar drawer over the content on first
    // paint; its scrim swallows content clicks until it is dismissed.
    if (await page.locator(".mobile-drawer-scrim").count()) {
      await page.keyboard.press("Escape");
      await page.locator(".mobile-drawer-scrim").waitFor({ state: "detached", timeout: 4000 }).catch(() => {});
    }
    await page.locator("a.page-ref", { hasText: "Tine" }).first().click();
    await page.waitForSelector(".pdf-link", { timeout: 8000 });
    await page.locator(".pdf-link").first().click();
    await page.waitForSelector('button[title="Close PDF"]', { timeout: 8000 });
    await sleep(500);
    const close = page.locator('button[title="Close PDF"]');
    const box = await close.boundingBox();
    const closeReachable = box != null &&
      box.x >= 0 && box.y >= 0 &&
      box.x + box.width <= PHONE.width && box.y + box.height <= PHONE.height;
    await page.screenshot({ path: outputPath });
    return closeReachable;
  } finally {
    await context.close();
  }
}

try {
  if (!process.env.TINE_PDF_MOBILE_AFTER_URL) {
    server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "inherit" });
    await waitForServer(afterUrl);
  }

  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  try {
    if (baselineUrl) {
      const beforeReachable = await openPdfAtPhoneWidth(browser, baselineUrl, `${OUT}/pdf-mobile-before.png`);
      if (beforeReachable) throw new Error("baseline Close button was unexpectedly reachable");
      console.log("OK    pdf-mobile-before (Close clipped)");
    } else {
      console.log("SKIP  pdf-mobile-before (set TINE_PDF_MOBILE_BASELINE_URL for comparison)");
    }

    const afterReachable = await openPdfAtPhoneWidth(browser, afterUrl, `${OUT}/pdf-mobile-after.png`);
    if (!afterReachable) throw new Error("patched Close button is outside the 390px viewport");
    console.log("OK    pdf-mobile-after (Close reachable)");
  } finally {
    await browser.close();
  }
} finally {
  server?.kill("SIGTERM");
}

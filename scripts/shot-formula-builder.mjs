// Screenshot the Sheets formula builder popup against the built mock frontend.
// Usage: npm run build && node scripts/shot-formula-builder.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const OUT = "screenshots";
// Dynamic ESM chunks + the mock wasm don't resolve under file://, so serve the
// built dist over http (same pattern as the other shot-*.mjs harnesses).
const PORT = 5231;

fs.mkdirSync(OUT, { recursive: true });

async function waitForServer(url) {
  for (let i = 0; i < 60; i++) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(250);
  }
  throw new Error("vite preview did not start");
}

async function openBuilder(page, url) {
  await page.goto(url);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 3000 });
  await page.locator(".switcher-input").fill("Sheets demo");
  await sleep(350);
  await page.locator(".switcher-row").first().click();
  await page.waitForSelector(".sheet-table", { timeout: 5000 });
  await page.locator(".sheet-table").first().scrollIntoViewIfNeeded();
  await sleep(300);
  await page.evaluate(() => {
    const header = [...document.querySelectorAll(".sheet-field-header")]
      .find((el) => (el.textContent || "").includes("due-soon"));
    if (!header) throw new Error("missing due-soon formula header");
    header.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 440, clientY: 280 }));
  });
  await sleep(250);
  await page.evaluate(() => {
    const item = [...document.querySelectorAll(".ctx-item")]
      .find((el) => /Edit formula/.test(el.textContent || ""));
    if (!item) throw new Error("missing Edit formula item");
    item.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });
  await page.waitForSelector(".formula-builder-if", { timeout: 3000 });
  await sleep(250);
}

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
try {
  const url = `http://localhost:${PORT}/`;
  await waitForServer(url);
  const browser = await chromium.launch({
    args: [
      "--no-sandbox",
      "--disable-setuid-sandbox",
      "--no-zygote",
      "--single-process",
      "--disable-gpu",
      "--disable-dev-shm-usage",
    ],
  });
  const page = await browser.newPage({ viewport: { width: 1120, height: 820 }, deviceScaleFactor: 2 });
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));

  await openBuilder(page, url);
  await page.locator(".formula-editor").screenshot({ path: `${OUT}/formula-builder-light.png` });

  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await sleep(250);
  await page.locator(".formula-editor").screenshot({ path: `${OUT}/formula-builder-dark.png` });

  const checks = await page.evaluate(() => {
    const editor = document.querySelector(".formula-editor");
    const face = document.querySelector(".formula-builder-if");
    const rawToggle = document.querySelector(".formula-editor-raw-toggle");
    if (!editor || !face || !rawToggle) return null;
    const editorStyle = getComputedStyle(editor);
    const faceStyle = getComputedStyle(face);
    return {
      editorBg: editorStyle.backgroundColor,
      faceDisplay: faceStyle.display,
      text: face.textContent || "",
      rawToggle: rawToggle.textContent || "",
    };
  });
  if (!checks || !checks.faceDisplay.includes("flex") || !checks.text.includes("IF") || !checks.rawToggle.includes("raw")) {
    throw new Error(`formula builder screenshot state failed: ${JSON.stringify(checks)}`);
  }
  if (errors.length) throw new Error(errors.join("\n"));
  await browser.close();
  console.log(`wrote ${OUT}/formula-builder-light.png and ${OUT}/formula-builder-dark.png`);
} catch (e) {
  console.error(String(e));
  server.kill();
  process.exit(1);
} finally {
  server.kill();
}

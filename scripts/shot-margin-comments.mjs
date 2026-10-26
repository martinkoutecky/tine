// Visual check for margin dialogue slice 1 (vision §3.7): an agent-written block
// with a comment card (three-level thread), a repeated-phrase quote, a stale quote
// and an untouched block, in light and dark at wide, desktop and phone width.
// Zooming into the passage must keep the same threads and replies inline.
// Usage: npm run build && node scripts/shot-margin-comments.mjs [outDir]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { bulletlessFacts, checkBulletless } from "./lib/margin-bulletless.mjs";

const PORT = 5217;
const OUT = process.argv[2] ?? "scratch/margin-s1";
const CASES = [
  { name: "wide-light", mode: "light", width: 1440, height: 900 },
  { name: "wide-dark", mode: "dark", width: 1440, height: 900 },
  { name: "desktop-light", mode: "light", width: 900, height: 640 },
  { name: "desktop-dark", mode: "dark", width: 900, height: 640 },
  { name: "phone-light", mode: "light", width: 390, height: 844 },
  { name: "phone-dark", mode: "dark", width: 390, height: 844 },
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

try {
  mkdirSync(OUT, { recursive: true });
  await waitForServer(`http://localhost:${PORT}/?regressions`);
  browser = await chromium.launch();
  for (const testCase of CASES) {
    const page = await browser.newPage({ viewport: { width: testCase.width, height: testCase.height } });
    page.setDefaultTimeout(5000);
    const errors = [];
    page.on("console", (message) => message.type() === "error" && errors.push(message.text()));
    page.on("pageerror", (error) => errors.push(String(error)));
    await page.goto(`http://localhost:${PORT}/?regressions`);
    await page.waitForSelector(".ls-block", { timeout: 5000 });
    await page.evaluate((mode) => document.documentElement.setAttribute("data-theme", mode), testCase.mode);
    await page.keyboard.press("Control+k");
    await page.locator(".switcher-input").fill("Margin comments regression");
    await page.locator(".switcher-row").first().click();
    await page.getByText("This sentence was rewritten since.").waitFor({ timeout: 5000 });
    // Phone width opens the page drawer over the content; close it for the shot.
    if (await page.locator(".mobile-drawer-scrim").count()) {
      await page.keyboard.press("Escape");
      await page.locator(".mobile-drawer-scrim").waitFor({ state: "detached", timeout: 4000 }).catch(() => {});
    }
    await sleep(300);

    const facts = await page.evaluate(() => ({
      marks: [...document.querySelectorAll(".comment-quote-anchor")].map((el) => el.textContent),
      cards: document.querySelectorAll(".ls-block.comment-card").length,
      stale: [...document.querySelectorAll(".comment-quote.stale")].map((el) => el.textContent),
      chips: [...document.querySelectorAll(".author-chip")].map((el) => el.textContent),
      hiddenRows: [...document.querySelectorAll(".block-property-key")].map((el) => el.textContent)
        .filter((key) => ["quote", "quote-prefix", "quote-suffix", "author"].includes(key ?? "")),
      overflow: document.documentElement.scrollWidth > window.innerWidth,
    }));
    console.log(JSON.stringify({ case: testCase.name, ...facts }));
    if (facts.marks.join("|") !== "converges here because|the step size shrinks") throw new Error(`${testCase.name}: marks ${facts.marks}`);
    if (facts.cards !== 3) throw new Error(`${testCase.name}: ${facts.cards} comment cards`);
    if (facts.stale.length !== 1 || !facts.stale[0].includes("quoted text changed")) throw new Error(`${testCase.name}: stale ${facts.stale}`);
    if (facts.hiddenRows.length) throw new Error(`${testCase.name}: property rows shown for ${facts.hiddenRows}`);
    checkBulletless(testCase.name, await page.evaluate(bulletlessFacts), { expectCards: 3 });
    if (facts.overflow) throw new Error(`${testCase.name}: horizontal page overflow`);
    if (errors.length) throw new Error(`${testCase.name}: browser errors:\n${errors.join("\n")}`);
    await page.screenshot({ path: `${OUT}/${testCase.name}.png`, fullPage: false });
    if (testCase.width === 390) {
      if (await page.locator(".margin-column").count()) throw new Error(`${testCase.name}: phone drew a margin`);
      if (await page.locator(".page-blocks .comment-card").count() !== 3) throw new Error(`${testCase.name}: phone lost inline threads`);
    }
    // Zoom the passage, using the same bullet action as a reader. Its thread
    // and replies must stay inline at phone and desktop widths alike.
    const passage = page.locator(".page-blocks .ls-block").filter({ has: page.locator(".comment-quote-anchor") }).first();
    const replies = await page.locator(".comment-card").first().locator(".ls-block").count();
    if (!replies) throw new Error(`${testCase.name}: fixture lost its replies`);
    await passage.locator(":scope > .block-main .bullet-container").click();
    await page.locator(".zoomed-page").waitFor();
    if (await page.locator(".margin-column").count()) throw new Error(`${testCase.name}: zoom drew a margin`);
    if (await page.locator(".zoomed-block .comment-card").count() !== facts.cards) throw new Error(`${testCase.name}: zoom lost its threads`);
    if (await page.locator(".zoomed-block .comment-card .ls-block").count() !== replies) throw new Error(`${testCase.name}: zoom lost replies`);
    await page.screenshot({ path: `${OUT}/${testCase.name}-zoom.png`, fullPage: false });
    console.log(JSON.stringify({ case: `${testCase.name}-zoom`, inlineThread: true, replies }));
    await page.close();
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

// Routed browser proof for legacy threads and dated message outlines, at each tier.
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";

const port = 5217;
const url = process.env.TINE_SCREENSHOT_URL || `http://localhost:${port}`;
const out = process.argv[2] ?? "target/artifacts/margin-comments";
mkdirSync(out, { recursive: true });
const server = process.env.TINE_SCREENSHOT_URL ? null : spawn("npx", ["vite", "preview", "--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(url)).ok) break; } catch {}
    if (attempt === 60) throw new Error("Preview server did not start");
    await delay(100);
  }
  browser = await chromium.launch();
  for (const width of [1440, 900, 390]) for (const mode of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width, height: 900 } });
    page.setDefaultTimeout(5000);
    const errors = [];
    page.on("pageerror", error => errors.push(String(error)));
    await page.goto(`${url}/?regressions`);
    await page.locator(".ls-block").first().waitFor();
    await page.evaluate(mode => document.documentElement.setAttribute("data-theme", mode), mode);
    for (const [name, dated] of [["Margin comments regression", false], ["Message outlines regression", true]]) {
      await page.keyboard.press("Control+k");
      await page.locator(".switcher-input").fill(name);
      await page.locator(".switcher-row").first().click();
      if (await page.locator(".mobile-drawer-scrim").count()) await page.keyboard.press("Escape");
      const marker = page.locator(".comment-quote-anchor").filter({ hasText: dated ? "passage" : "converges here because" }).first();
      await marker.waitFor();
      for (const close of await page.locator(".toast-close").all()) await close.click();
      if (width < 960) {
        if (width <= 480) await page.locator(".margin-marker").first().click();
        else await marker.hover();
      }
      const card = page.locator(".comment-card").filter({ hasText: dated ? "The first message" : "Only if the loss is convex" }).first();
      await card.waitFor();
      await page.screenshot({ path: `${out}/${dated ? "messages" : "legacy"}-${width}-${mode}.png` });
      assert.equal(await card.locator(":scope > .block-main .bullet-container").count(), 0, "Message text stays bulletless");
      if (dated) {
        await card.locator(".comment-header").nth(1).waitFor();
        assert.equal(await card.locator(".comment-header").count(), 2, "Only messages have headers");
        assert.equal(await card.locator(".bullet-container").count(), 3, "Body blocks keep ordinary bullets");
        await card.locator(".comment-collapse").last().click();
        await page.getByText("An editable reply outline", { exact: true }).waitFor({ state: "detached" });
        await card.locator(".comment-collapse").last().click();
        await page.getByText("An editable reply outline", { exact: true }).waitFor();
        await card.locator(":scope > .comment-header .comment-collapse").click();
        await card.getByRole("button", { name: "1 reply", exact: true }).waitFor();
        await card.getByRole("button", { name: "1 reply", exact: true }).click();
        await page.getByText("Nested evidence", { exact: true }).waitFor();
      } else {
        await card.locator(".comment-header").nth(1).waitFor();
        assert.ok(await card.locator(".comment-header").count() > 1, "Legacy replies remain flat messages");
        assert.equal(await card.locator(".bullet-container").count(), 0, "Legacy replies stay bulletless");
      }
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, "No horizontal page overflow");
      await page.screenshot({ path: `${out}/${dated ? "messages" : "legacy"}-${width}-${mode}.png` });
      console.log(JSON.stringify({ width, mode, dated, passed: true }));
    }
    assert.deepEqual(errors, []);
    await page.close();
  }
} finally {
  await browser?.close(); server?.kill();
}

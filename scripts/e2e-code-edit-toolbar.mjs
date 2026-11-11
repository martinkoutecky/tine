// GH #879 items 2–3. Run against the built app with its synthetic browser graph:
// npm run build && node scripts/e2e-code-edit-toolbar.mjs
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { openJournals } from "./lib/e2e-navigation.mjs";

const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || "target/artifacts/og-codeblock-menu-879");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "browser-"));
// The app uses only the synthetic in-memory graph. Isolate server and browser
// homes too, so no launch can consult the developer's app state.
const env = { ...process.env, HOME: out, XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_DATA_HOME: path.join(out, "data"), XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state") };
for (const dir of ["config", "data", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
const port = Number(process.env.E2E_TOOLBAR_PORT || 5243);
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], { env, stdio: "ignore" });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  workingDiff: true,
  journey: "Code copy controls remain usable while editing on desktop hover and touch focus, on a routed page and the journal, in both editors", results: [] };
let browser, page, stage = "startup";
const visible = button => button.evaluate(el => el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })
  && getComputedStyle(el).pointerEvents !== "none");
async function waitVisible(button, expected) {
  await page.waitForFunction(([el, expected]) => (el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })
    && getComputedStyle(el).pointerEvents !== "none") === expected, [await button.elementHandle(), expected]);
}
async function openPage(name) {
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill(name);
  await page.locator(".switcher-row").filter({ hasText: name }).first().click();
}
try {
  for (let i = 0; i < 80; i++) {
    try { if ((await fetch(`http://127.0.0.1:${port}`)).ok) break; } catch {}
    await sleep(250);
  }
  browser = await chromium.launch({ env });
  for (const engine of ["textarea", "cm6"]) for (const mobile of [false, true]) {
    const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 780 } : { width: 1100, height: 850 },
      hasTouch: mobile, isMobile: mobile, permissions: ["clipboard-read", "clipboard-write"] });
    page = await context.newPage();
    page.setDefaultTimeout(10000);
    const errors = [];
    page.on("pageerror", error => errors.push(String(error)));
    await page.goto(`http://127.0.0.1:${port}/?regressions`);
    await page.locator(".ls-block").first().waitFor();
    if (engine === "cm6") {
      await page.keyboard.press("Control+,");
      await page.locator(".settings-search-input").fill("CodeMirror");
      await page.locator(".settings-search-result").first().click();
      await page.locator(".settings-field").filter({ hasText: "Experimental: CodeMirror editor" }).getByRole("switch").click();
      await page.keyboard.press("Escape");
    }
    for (const surface of ["page", "journal"]) {
      stage = `${engine}-${mobile ? "mobile" : "desktop"}-${surface}`;
      if (surface === "page") await openPage("CM6 code highlight regression");
      else await openJournals(page);
      const card = page.locator(".main-content pre.code-block").filter({ hasText: surface === "page" ? "const greeting" : "fn main" }).first();
      await card.scrollIntoViewIfNeeded();
      const id = await card.locator("xpath=ancestor::*[@data-block-id][1]").getAttribute("data-block-id");
      const row = page.locator(`.main-content [data-block-id="${id}"]`).first();
      if (mobile) assert.equal(await visible(card.locator(".code-copy")), false, `${stage}: hidden before editing`);
      const code = card.locator("code");
      if (mobile) await code.tap({ position: { x: 80, y: 10 } });
      else await code.click({ position: { x: 80, y: 10 } });
      const editor = row.locator(engine === "cm6" ? ".cm-content.code-edit" : "textarea.code-edit");
      await editor.waitFor();
      await page.keyboard.press("Control+End");
      await page.keyboard.type(" // toolbar journey");
      const expected = engine === "cm6" ? await editor.innerText() : await editor.inputValue();
      const copy = row.getByRole("button", { name: "Copy code", exact: true });
      assert.equal(await copy.count(), 1, `${stage}: copy control exists while editing`);
      await page.mouse.move(1, 1);
      if (!mobile) {
        await waitVisible(copy, false);
        assert.equal(await visible(copy), false, `${stage}: hidden until hover`);
        // Reach the top-right corner with the real pointer while the caret stays in code.
        const box = await copy.boundingBox();
        await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      }
      await waitVisible(copy, true);
      await page.screenshot({ path: path.join(out, `${stage}-editing.png`) });
      if (mobile) await copy.tap(); else await copy.click();
      await page.waitForFunction(expected => navigator.clipboard.readText().then(text => text === expected), expected);
      assert.equal(await editor.evaluate(el => el === document.activeElement), true, `${stage}: copying keeps the caret in code`);
      // Leave code by entering a different block, not merely blurring the window.
      const prose = page.locator(".main-content .block-content").filter({ hasText: surface === "page" ? "Click a code body" : "Started the" }).first();
      // The journal sentence also contains links; enter its plain opening text.
      if (mobile) await prose.tap({ position: { x: 10, y: 12 } });
      else await prose.click({ position: { x: 10, y: 12 } });
      await row.locator("pre.code-block").waitFor();
      await row.evaluate(el => el.scrollIntoView({ block: "center" }));
      await page.mouse.move(1, 1);
      await waitVisible(row.locator(".code-copy"), false);
      assert.equal(await visible(row.locator(".code-copy")), false, `${stage}: hidden after caret leaves`);
      await page.screenshot({ path: path.join(out, `${stage}-left.png`) });
      proof.results.push({ stage, copiedCurrentBody: true, keptCaret: true, hiddenAfterLeaving: true });
      await page.keyboard.press("Escape");
    }
    assert.deepEqual(errors, []);
    await context.close();
  }
  proof.outcome = "PASS";
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  console.log(`PASS GH #879: ${out}`);
} catch (error) {
  try { await page?.screenshot({ path: path.join(out, "failure.png") }); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "The current code body can be copied while the caret remains inside; controls hide after it leaves",
    observed: String(error.stack), classification: error.code === "ERR_ASSERTION" ? "product" : "ambiguous; inspect screenshot and failing assertion" }, null, 2) + "\n");
  throw error;
} finally {
  await browser?.close();
  server.kill();
}

import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { chromium } from "playwright";
import { createServer } from "vite";

// Actual compositor input; dispatchEvent(TouchEvent) cannot prove native scrolling.
async function pan(page, target, direction = -1) {
  await target.scrollIntoViewIfNeeded();
  const box = await target.boundingBox();
  assert.ok(box && box.width > 100, "Pan target is visible and wide enough");
  const x = box.x + (direction < 0 ? Math.min(box.width - 20, 250) : 20);
  const y = box.y + box.height / 2;
  const cdp = await page.context().newCDPSession(page);
  try {
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y }] });
    for (let step = 1; step <= 12; step++) {
      await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: x + direction * step * 10, y }] });
      await page.evaluate(() => new Promise(requestAnimationFrame));
    }
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  } finally { await cdp.detach(); }
}

export async function checkMathTouch(root) {
  fs.mkdirSync(path.join(root, "target/artifacts/math-touch-pan"), { recursive: true });
  const out = fs.mkdtempSync(path.join(root, "target/artifacts/math-touch-pan/touch-"));
  const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(), outcomes: [] };
  const server = await createServer({ server: { port: 0, strictPort: false,
    fs: { allow: [root, fs.realpathSync(path.join(root, "node_modules/katex")), fs.realpathSync(path.join(root, "node_modules/inter-ui"))] } }, logLevel: "error",
    plugins: [{ name: "math-touch-fixture", configureServer(server) {
      server.middlewares.use("/__math_touch", async (_req, res) => {
        res.setHeader("Content-Type", "text/html");
        res.end(await server.transformIndexHtml("/__math_touch", '<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1"><script type="module" src="/scripts/lib/math-touch-fixture.tsx"></script>'));
      });
    } }],
  });
  let browser, page;
  try {
    await server.listen();
    browser = await chromium.launch();
    proof.chromium = browser.version();
    page = await browser.newPage({ viewport: { width: 390, height: 900 }, hasTouch: true, isMobile: true });
    page.setDefaultTimeout(15000);
    page.on("pageerror", error => (proof.errors ??= []).push(String(error)));
    await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/__math_touch`);
    await page.addStyleTag({ content: "html {overflow-x:hidden} body {display:block;overflow:auto;padding:12px;width:390px;max-width:390px;box-sizing:border-box} [data-probe] {display:block;width:340px;margin:20px 0} .swipe-start {display:block;height:36px} .sheet-board {width:340px} td {white-space:nowrap}" });
    await page.locator('[data-probe="display"] .katex').waitFor();
    await page.evaluate(() => document.fonts.ready);
    // Keep the current rules and separately probe the proposed exemption removal.
    for (const exempt of [true, false]) {
      const override = exempt ? null : await page.addStyleTag({ content: "html[data-touch-gestures] .block-main {touch-action:pan-y pinch-zoom}" });
      for (const [name, selector] of [["inline", ".math > span"], ["display", ".math > span"], ["code", ".code-block"], ["table", ".md-table-wrap"], ["sheet", ".sheet-scroll"]]) {
        console.log(`Touch: ${name}, exempt=${exempt}`);
        proof.stage = `${name}, exempt=${exempt}`;
        const row = page.locator(`[data-probe="${name}"]`), scroller = row.locator(selector);
        await scroller.evaluate(el => { el.scrollLeft = 0; });
        await pan(page, scroller);
        await page.waitForFunction(selector => document.querySelector(selector).scrollLeft > 0, `[data-probe="${name}"] ${selector}`);
        const scroll = await scroller.evaluate(el => el.scrollLeft);
        assert.equal(await row.getAttribute("data-committed"), null, `${name}: panning content must not swipe the block`);
        await row.evaluate(el => { delete el.dataset.committed; });
        await pan(page, row.locator(".swipe-start"), 1);
        const swipe = await row.getAttribute("data-committed");
        assert.equal(swipe, "indent", `${name}: swipe outside the scroller remains available`);
        await row.evaluate(el => { delete el.dataset.committed; });
        proof.outcomes.push({ name, exempt, scroll, swipe });
      }
      await override?.evaluate(el => el.remove());
    }
    for (const id of ["inline", "display"]) {
      const card = page.locator(`.sheet-board-card[data-block-id="${id}"]`);
      proof.stage = `board-${id}`;
      await card.scrollIntoViewIfNeeded();
      await card.locator(".katex").waitFor();
      assert.equal(await card.locator(".math-display").count(), id === "display" ? 1 : 0,
        "Exercise the requested formula mode inside a real card title");
      assert.equal(await card.evaluate(el => el.classList.contains("sheet-board-card-static")), false, "Use a movable card");
      await card.evaluate(el => {
        el.addEventListener("pointercancel", () => { el.dataset.cancelled = "true"; }, { once: true });
      });
      const scroller = card.locator(".math > span");
      const legacyBlocked = await scroller.evaluate(el => {
        const actions = [];
        for (let ancestor = el; ancestor; ancestor = ancestor.parentElement) {
          const action = getComputedStyle(ancestor).touchAction;
          if (action === "none" || (action.includes("pan-y") && !action.includes("pan-x"))) actions.push(action);
        }
        return actions;
      });
      const column = await card.getAttribute("data-sheet-column-id");
      await pan(page, scroller);
      await page.waitForFunction(id => document.querySelector(`.sheet-board-card[data-block-id="${id}"] .math > span`).scrollLeft > 0, id);
      assert.equal(await card.getAttribute("data-cancelled"), "true", "Native pan cancels card dragging");
      assert.equal(await page.locator(".sheet-board-card-dragging, .sheet-board-drag-ghost").count(), 0);
      assert.equal(await card.getAttribute("data-sheet-column-id"), column, "Panning does not move a card to another column");
      proof.outcomes.push({ name: `board-${id}`, scroll: await scroller.evaluate(el => el.scrollLeft), cancelled: true, legacyBlocked });
    }
    proof.layout = await page.evaluate(() => {
      const host = document.createElement("div");
      const formula = document.querySelector(".math");
      for (let i = 0; i < 120; i++) host.append(formula.cloneNode(true), document.createElement("br"));
      document.body.append(host);
      host.style.width = "640px";
      const coldStart = performance.now();
      void host.offsetHeight;
      const coldLayoutMs = performance.now() - coldStart;
      const samples = [];
      for (const width of [390, 640, 390, 640, 390, 640, 390, 640]) {
        const start = performance.now();
        host.style.width = `${width}px`;
        void host.offsetHeight;
        samples.push(performance.now() - start);
      }
      host.remove();
      return { formulas: 120, coldLayoutMs, samplesMs: samples, maxMs: Math.max(...samples) };
    });
    assert.ok(proof.layout.maxMs < 500, "Math-heavy pane layout stays below the UI stall budget");
    await page.screenshot({ path: path.join(out, "board.png") });
    proof.outcome = "PASS";
  } catch (error) {
    proof.outcome = "FAIL";
    proof.expected = "Touch pans scroll formula/code/table/sheet content and cancel movable-card dragging";
    proof.observed = String(error.stack);
    proof.classification = error.code === "ERR_ASSERTION" ? "product" : "ambiguous";
    await page?.screenshot({ path: path.join(out, "failure.png") }).catch(() => {});
    throw error;
  } finally {
    fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
    console.log(`${proof.outcome}: ${out}`);
    await browser?.close();
    await server.close();
  }
}

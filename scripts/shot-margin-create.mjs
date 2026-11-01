// GH #110: real routed-page layout, selection toolbar / rendered-text menu -> first comment, and
// consistent author/body separation for root, own, other and nested replies.
// Usage: node scripts/shot-margin-create.mjs [artifactDir]; MARGIN_DEV=1 uses Vite dev.
import { chromium } from "playwright";
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const out = process.argv[2] ?? "target/artifacts/fix-beta-margin-110/browser";
const port = Number(process.env.MARGIN_SHOT_PORT || 5227);
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
mkdirSync(out, { recursive: true });
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", ...(process.env.MARGIN_DEV ? [] : ["preview"]), "--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
const problems = [];
const evidence = [];
try {
  for (let i = 0; ; i++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (i === 120) throw new Error("Vite did not start");
    await sleep(250);
  }
  browser = await chromium.launch();
  for (const width of [1280, 820]) for (const mode of ["light", "dark"]) for (const route of ["editor", "rendered"]) {
    const page = await browser.newPage({ viewport: { width, height: 900 } });
    page.setDefaultTimeout(15_000);
    page.on("pageerror", error => problems.push(String(error)));
    const open = async name => {
      await page.goto(`http://localhost:${port}/?regressions`);
      await page.locator(".ls-block").first().waitFor();
      await page.evaluate(m => { document.documentElement.dataset.theme = m; }, mode);
      await page.keyboard.press("Control+k");
      await page.locator(".switcher-input").fill(name);
      await page.locator(".switcher-row").first().click();
      await page.locator(".page-title").filter({ hasText: name }).first().waitFor();
      await page.mouse.move(2, 2);
      await sleep(300);
    };
    await open("Margin creation regression");
    const body = page.locator(".page-blocks .block-content").filter({ hasText: "Discuss these words here" }).first();
    if (route === "editor") {
      await body.click();
      await page.locator(".block-main.editing textarea").evaluate(el => { el.setSelectionRange(8, 19); el.dispatchEvent(new Event("select", { bubbles: true })); });
    } else {
      await body.evaluate(el => {
        const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
        let node;
        while ((node = walker.nextNode())) {
          const at = node.textContent.indexOf("these words");
          if (at >= 0) { getSelection().setBaseAndExtent(node, at, node, at + 11); break; }
        }
      });
      await body.dispatchEvent("contextmenu", { button: 2, clientX: 500, clientY: 520 });
    }
    const button = route === "editor" ? page.locator('.sel-toolbar [data-selection-action="comment"]') : page.locator(".ctx-item").filter({ hasText: /^Comment on selection$/ });
    await button.waitFor();
    await page.evaluate(() => {
      const parent = [...document.querySelectorAll(".page-blocks .ls-block")].find(el => el.querySelector(":scope > .block-main .block-content, :scope > .block-main textarea")?.textContent.includes("Discuss these words here") || el.querySelector(":scope > .block-main textarea")?.value === "Discuss these words here");
      const scroller = parent.closest(".main-content");
      window.__marginFrames = [];
      window.__marginBefore = { top: parent.querySelector(":scope > .block-main").getBoundingClientRect().top, scroll: scroller.scrollTop, id: parent.dataset.blockId };
      const tick = () => {
        const edit = document.querySelector(".block-main.editing textarea");
        window.__marginFrames.push({ top: parent.querySelector(":scope > .block-main").getBoundingClientRect().top, rowHeight: parent.querySelector(":scope > .block-main").getBoundingClientRect().height,
          scroll: scroller.scrollTop, editorTop: edit?.getBoundingClientRect().top,
          editorParent: edit?.closest(".ls-block")?.dataset.blockId, threadTop: edit?.closest(".margin-thread")?.getBoundingClientRect().top, margin: !!edit?.closest(".margin-thread") });
        if (window.__marginFrames.length < 45) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    await button.click();
    await page.locator(".comment-in-thread .editing textarea").waitFor();
    await sleep(900);
    const frames = await page.evaluate(() => ({ before: window.__marginBefore, frames: window.__marginFrames }));
    const jump = Math.max(...frames.frames.map(f => Math.abs(f.top - frames.before.top)));
    const newEditor = frames.frames.filter(f => f.editorParent && f.editorParent !== frames.before.id);
    const editorJump = newEditor.length ? Math.max(...newEditor.map(f => f.editorTop)) - Math.min(...newEditor.map(f => f.editorTop)) : null;
    evidence.push({ width, mode, route, jump, editorJump, ...frames });
    writeFileSync(`${out}/evidence.json`, JSON.stringify({ testedCommit, evidence, problems }, null, 2));
    await page.screenshot({ path: `${out}/create-${width}-${mode}-${route}.png` });
    // A new comment directly under/beside a visible single-line passage needs
    // no vertical navigation. Check every painted frame, not just the end state.
    if (jump > 2) problems.push(`${width}-${mode}: passage jumped ${jump}px`);
    if (editorJump === null || editorJump > 2) problems.push(`${width}-${mode}-${route}: new editor jumped ${editorJump}px`);
    if (frames.frames.some(f => f.margin && Math.abs(f.threadTop - f.top) > f.rowHeight)) problems.push(`${width}-${mode}-${route}: new margin thread appeared away from its passage`);
    const active = await page.evaluate(() => {
      const el = document.querySelector(".comment-in-thread .editing textarea");
      return el ? { focused: el === document.activeElement, top: el.getBoundingClientRect().top, bottom: el.getBoundingClientRect().bottom, view: el.closest(".main-content").getBoundingClientRect().toJSON() } : null;
    });
    if (!active?.focused || active.top < active.view.top || active.bottom > active.view.bottom) problems.push(`${width}-${mode}: new comment editor lost/offscreen ${JSON.stringify(active)}`);
    await open("Margin position regression");
    const identity = await page.evaluate(() => [...document.querySelectorAll(".comment-in-thread")].map(card => {
      const author = [...card.querySelectorAll(".author-chip")].find(el => el.closest(".ls-block") === card);
      const body = card.querySelector(":scope > .block-main");
      const quote = card.querySelector(":scope > .comment-quote");
      return { id: card.dataset.blockId, author: author?.textContent, separate: !!author && author.getBoundingClientRect().bottom <= body.getBoundingClientRect().top,
        quoteSeparate: !quote || quote.getBoundingClientRect().bottom <= author?.getBoundingClientRect().top };
    }));
    if (identity.length < 6 || identity.some(c => !c.separate || !c.quoteSeparate)) problems.push(`${width}-${mode}: responses lack distinct author/body rows ${JSON.stringify(identity)}`);
    evidence.push({ width, mode, route, identity });
    await page.screenshot({ path: `${out}/identity-${width}-${mode}.png` });
    await page.close();
  }
  writeFileSync(`${out}/evidence.json`, JSON.stringify({ testedCommit, evidence, problems }, null, 2));
  if (problems.length) throw new Error(problems.join("\n"));
  console.log(`GH #110 creation and identity checks passed; screenshots in ${out}`);
} catch (error) {
  const page = browser?.contexts().at(-1)?.pages().at(-1);
  await page?.screenshot({ path: `${out}/failure.png` }).catch(() => {});
  writeFileSync(`${out}/failure.json`, JSON.stringify({ testedCommit, journey: "selection toolbar -> first comment above long subtree; identify responses",
    expected: "Passage stays in view, focused new comment stays beside/below it, and each response has a separate author row",
    observed: String(error), classification: problems.length ? "product" : "harness", evidence: "evidence.json / failure.png" }, null, 2));
  throw error;
} finally {
  await browser?.close();
  server.kill();
}

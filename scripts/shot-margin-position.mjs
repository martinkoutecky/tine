// Geometry check for MARGIN-POS / MARGIN-WIDTH / MARGIN-AUTHOR (2026-10-31): in
// the margin each thread sits beside its passage, or as close below it as the
// threads above allow (Google Docs), and stays there when the commented block
// enters and leaves editing; a pane with room to spare widens the column;
// someone else's comment has its own background. On a narrow pane a block's
// comments show right under it, before its other children (GH #110).
// Usage: npm run build && node scripts/shot-margin-position.mjs [outDir]
// CM6=1 also turns on the experimental CodeMirror editor where the build has one.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = Number(process.env.MARGIN_SHOT_PORT || 5219);
const OUT = process.argv[2] ?? "scratch/margin-position";
const PAGES = ["Margin position regression", "Margin stacking regression", "Margin comments regression"];
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
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

const frames = (tab) => tab.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

function threadFacts() {
  const column = document.querySelector(".margin-column");
  const outline = document.querySelector(".page-blocks").getBoundingClientRect();
  return {
    columnWidth: column ? column.getBoundingClientRect().width : 0,
    outlineWidth: outline.width,
    outlineRight: outline.right,
    overflow: document.documentElement.scrollWidth > window.innerWidth,
    threads: [...document.querySelectorAll(".margin-thread")].map((el) => {
      const id = el.dataset.threadId;
      const mark = document.querySelector(`.page-blocks .comment-quote-anchor[data-comment-id="${id}"]`);
      const r = el.getBoundingClientRect();
      return { id, top: r.top, bottom: r.bottom, left: r.left, anchor: mark ? mark.getBoundingClientRect().top : null };
    }),
  };
}

function checkThreads(name, f) {
  const fail = (why) => { throw new Error(`${name}: ${why}\n${JSON.stringify(f)}`); };
  if (f.overflow) fail("horizontal page overflow");
  const sorted = [...f.threads].sort((a, b) => a.top - b.top);
  for (let i = 1; i < sorted.length; i++) {
    if (sorted[i].top < sorted[i - 1].bottom) fail(`threads ${sorted[i - 1].id} and ${sorted[i].id} overlap`);
  }
  for (const [i, t] of sorted.entries()) {
    if (t.left < f.outlineRight) fail(`thread ${t.id} overlaps the outline`);
    if (t.anchor === null) continue;
    if (t.top < t.anchor - 1) fail(`thread ${t.id} sits above its passage`);
    // Beside its passage unless the thread above is in the way.
    const blocked = i > 0 && sorted[i - 1].bottom + 8 > t.anchor + 1;
    if (!blocked && Math.abs(t.top - t.anchor) > 1) fail(`thread ${t.id} is ${Math.round(t.top - t.anchor)}px from its free passage`);
  }
}

async function open(tab, name, mode) {
  await tab.goto(`http://localhost:${PORT}/?regressions`);
  await tab.waitForSelector(".ls-block");
  await tab.evaluate((m) => document.documentElement.setAttribute("data-theme", m), mode);
  await tab.keyboard.press("Control+k");
  await tab.locator(".switcher-input").fill(name);
  await tab.locator(".switcher-row").first().click();
  await tab.locator(".page-title").filter({ hasText: name }).first().waitFor();
  // Park the pointer: a hovered thread shows its reply box and grows.
  await tab.mouse.move(2, 2);
  await sleep(300);
}

try {
  mkdirSync(OUT, { recursive: true });
  await waitForServer(`http://localhost:${PORT}/?regressions`);
  browser = await chromium.launch();
  for (const mode of ["light", "dark"]) for (const size of [{ width: 1920, height: 1000 }, { width: 1280, height: 900 }]) {
    const tab = await browser.newPage({ viewport: size });
    tab.setDefaultTimeout(5000);
    const errors = [];
    tab.on("pageerror", (error) => errors.push(String(error)));
    await tab.goto(`http://localhost:${PORT}/?regressions`);
    await tab.waitForSelector(".ls-block");
    await tab.evaluate((m) => document.documentElement.setAttribute("data-theme", m), mode);
    for (const page of PAGES) {
      const name = `${page.split(" ")[1]}-${size.width}-${mode}`;
      await open(tab, page, mode);
      const f = await tab.evaluate(threadFacts);
      if (!f.threads.length) throw new Error(`${name}: no margin threads`);
      checkThreads(name, f);
      if (size.width === 1920 && f.columnWidth <= 300) throw new Error(`${name}: column stayed ${f.columnWidth}px on a wide pane`);
      if (size.width === 1280 && f.columnWidth < 279) throw new Error(`${name}: column ${f.columnWidth}px below the minimum`);
      await tab.screenshot({ path: `${OUT}/${name}.png` });
      // Enter and leave edit on every commented block: no thread moves.
      const commented = await tab.evaluate(() => [...document.querySelectorAll(".page-blocks .ls-block")]
        .filter((el) => el.querySelector(":scope > .block-main .margin-marker")).map((el) => el.dataset.blockId));
      for (const id of commented) {
        const before = await tab.evaluate(threadFacts);
        await tab.locator(`.page-blocks .ls-block[data-block-id="${id}"] > .block-main .block-content`).first().click({ position: { x: 4, y: 4 } });
        await tab.locator(".editing").first().waitFor();
        await frames(tab);
        await sleep(150);
        const during = await tab.evaluate(threadFacts);
        if (id === commented[0]) await tab.screenshot({ path: `${OUT}/${name}-editing.png` });
        await tab.keyboard.press("Escape");
        await tab.mouse.move(2, 2);
        await frames(tab);
        await sleep(150);
        const after = await tab.evaluate(threadFacts);
        checkThreads(`${name}-after-edit-${id}`, after);
        // Threads of the edited block stay put; others may move only by the
        // row's own height change, which the probe blocks avoid (one-line raw).
        for (const t of before.threads) {
          const d = during.threads.find((x) => x.id === t.id);
          const a = after.threads.find((x) => x.id === t.id);
          const jump = Math.max(Math.abs((d?.top ?? t.top) - t.top), Math.abs((a?.top ?? t.top) - t.top));
          console.log(JSON.stringify({ case: name, edited: id.slice(-4), thread: t.id.slice(-4), duringDy: Math.round((d?.top ?? NaN) - t.top), afterDy: Math.round((a?.top ?? NaN) - t.top) }));
          if (jump > 24) throw new Error(`${name}: thread ${t.id} jumped ${Math.round(jump)}px when block ${id} was edited`);
        }
      }
      if (page.startsWith("Margin position")) {
        const tints = await tab.evaluate(() => [...document.querySelectorAll(".margin-thread .ls-block.comment-in-thread")].map((el) => ({
          tint: [...el.classList].find((c) => c.startsWith("author-tint-")) ?? null,
          bg: getComputedStyle(el.querySelector(":scope > .block-main")).backgroundColor,
        })));
        const own = tints.filter((t) => !t.tint);
        const others = tints.filter((t) => t.tint);
        if (!own.length || others.length < 3) throw new Error(`${name}: tints ${JSON.stringify(tints)}`);
        for (const o of others) if (own.some((y) => y.bg === o.bg)) throw new Error(`${name}: another author's reply has your background ${JSON.stringify(tints)}`);
      }
    }
    if (errors.length) throw new Error(`browser errors:\n${errors.join("\n")}`);
    await tab.close();
  }
  // Narrow: inline, comment directly under its block.
  for (const mode of ["light", "dark"]) {
    const tab = await browser.newPage({ viewport: { width: 820, height: 900 } });
    tab.setDefaultTimeout(5000);
    await tab.goto(`http://localhost:${PORT}/?regressions`);
    await tab.waitForSelector(".ls-block");
    await tab.evaluate((m) => document.documentElement.setAttribute("data-theme", m), mode);
    await open(tab, PAGES[0], mode);
    const order = await tab.evaluate(() => {
      const first = document.querySelector(".page-blocks .ls-block");
      return [...first.querySelectorAll(".ls-block")].map((el) => el.querySelector(".block-content")?.textContent.trim().slice(0, 14));
    });
    console.log(JSON.stringify({ case: `narrow-${mode}`, order }));
    if (!order[0]?.startsWith("Which source")) throw new Error(`narrow-${mode}: inline order ${JSON.stringify(order)}`);
    await tab.screenshot({ path: `${OUT}/narrow-${mode}.png` });
    await tab.close();
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

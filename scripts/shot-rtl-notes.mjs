// GH #823: screenshots and layout checks for right-to-left notes, which jsdom cannot see.
// usage: node scripts/shot-rtl-notes.mjs [out-dir]; env THEME=light|dark (default: both).
// Exits 1 when a block's direction or side differs between the view and either editor, when the
// mirrored outline leaves its bullets or indentation on the wrong side, or when an arrow key
// leaves a block in the direction opposite to the one on screen.
import { spawn } from "node:child_process";
import { closeSync, mkdirSync, openSync } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { chromium } from "playwright";

const out = resolve(process.argv[2] || "target/artifacts/rtl-notes");
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const log = openSync(`${out}/vite.log`, "w");
const server = spawn("npx", ["vite", "--port", String(port), "--strictPort"], { stdio: ["ignore", log, log], detached: true });
closeSync(log);
const failures = [];
const check = (ok, what) => { if (!ok) failures.push(what); };
const EXPECT = { fa: "rtl", "fa-child": "rtl", he: "rtl", "en-child": "ltr", todo: "rtl", ref: "rtl", ar: "rtl", en: "ltr", mark: "rtl", list: "rtl" };
let browser;
try {
  for (let a = 0; ; a++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (a === 600) throw new Error("Vite did not start");
    await delay(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const theme of process.env.THEME ? [process.env.THEME] : ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width: 800, height: 900 } });
    page.on("pageerror", e => failures.push(`page error: ${e.message}`));
    await page.goto(`http://localhost:${port}/scripts/fixtures/rtl-notes.html`, { timeout: 120_000 });
    await page.waitForSelector('[data-block-id="fa"] .block-content');
    await page.evaluate(t => { document.documentElement.dataset.theme = t; document.documentElement.classList.toggle("dark", t === "dark"); document.body.style.background = t === "dark" ? "#1e1e1e" : "#fff"; document.body.style.color = t === "dark" ? "#ddd" : "#222"; }, theme);
    const probe = (id, editor) => page.evaluate(([i, e]) => {
      const row = document.querySelector(`[data-block-id="${i}"] > .block-main`);
      const el = row.querySelector(e === "view" ? ".block-content" : e === "textarea" ? "textarea" : ".cm-content");
      const cs = getComputedStyle(el), r = el.getBoundingClientRect();
      return { dir: cs.direction, align: cs.textAlign, left: r.left, right: r.right };
    }, [id, editor]);
    const open = async (id, cm6, caret = 0) => { await page.evaluate(([i, o, c]) => { window.endEdit(); window.setCm6EditorEnabled(o); window.startEditing(i, c); }, [id, cm6, caret]); await page.waitForSelector(`[data-block-id="${id}"] ${cm6 ? ".cm-content" : "textarea"}`); };
    for (const layout of ["ltr", "rtl"]) {
      await page.evaluate(l => { window.endEdit(); window.setNoteLayoutRtl(l === "rtl"); }, layout);
      await delay(100);
      await page.screenshot({ path: `${out}/${theme}-${layout}-view.png`, fullPage: true });
      const geo = await page.evaluate(() => {
        const box = s => document.querySelector(s).getBoundingClientRect();
        return { bullet: box('[data-block-id="fa"] > .block-main .bullet-container'), body: box('[data-block-id="fa"] > .block-main .block-content'),
          child: box('[data-block-id="fa-child"] > .block-main .block-content'), parent: box('[data-block-id="fa"] > .block-main .block-content') };
      });
      if (layout === "rtl") {
        check(geo.bullet.left >= geo.body.right - 1, `${theme}: mirrored bullet sits right of its text`);
        check(geo.child.right < geo.parent.right - 10, `${theme}: mirrored child indents from the right`);
      } else {
        check(geo.bullet.right <= geo.body.left + 1, `${theme}: default bullet sits left of its text`);
        check(geo.child.left > geo.parent.left + 10, `${theme}: default child indents from the left`);
      }
      for (const [id, want] of Object.entries(EXPECT)) {
        const view = await probe(id, "view");
        check(view.dir === want, `${theme}/${layout}/${id}: view is ${view.dir}, want ${want}`);
        for (const [name, cm6] of [["textarea", false], ["cm6", true]]) {
          await open(id, cm6);
          const ed = await probe(id, name);
          check(ed.dir === want, `${theme}/${layout}/${id}: ${name} is ${ed.dir}, want ${want}`);
          check(Math.abs(ed.left - view.left) <= 1 && Math.abs(ed.right - view.right) <= 1, `${theme}/${layout}/${id}: ${name} box moved from the view`);
          if (id === "fa" || id === "todo") await page.screenshot({ path: `${out}/${theme}-${layout}-${id}-${name}.png`, fullPage: true });
        }
      }
      // Arrows keep their on-screen meaning at a block's edges.
      for (const cm6 of [false, true]) {
        const label = cm6 ? "cm6" : "textarea", editing = () => page.evaluate(() => document.activeElement?.closest("[data-block-id]")?.dataset.blockId);
        await open("todo", cm6, 0); await page.waitForTimeout(60);
        await page.keyboard.press("ArrowLeft"); await page.waitForTimeout(60);
        check(await editing() === "todo", `${theme}/${layout}/${label}: ArrowLeft at a right-to-left block's start moves into the text`);
        await open("todo", cm6, 0); await page.waitForTimeout(60);
        await page.keyboard.press("ArrowRight"); await page.waitForTimeout(100);
        check(await editing() === "en-child", `${theme}/${layout}/${label}: ArrowRight at a right-to-left block's start leaves backward (got ${await editing()})`);
        await open("todo", cm6, 1e9); await page.waitForTimeout(60);
        await page.keyboard.press("ArrowLeft"); await page.waitForTimeout(100);
        check(await editing() === "ref", `${theme}/${layout}/${label}: ArrowLeft at a right-to-left block's end leaves forward (got ${await editing()})`);
        await open("en", cm6, 0); await page.waitForTimeout(60);
        await page.keyboard.press("ArrowLeft"); await page.waitForTimeout(100);
        check(await editing() === "ar", `${theme}/${layout}/${label}: ArrowLeft at a left-to-right block's start leaves backward (got ${await editing()})`);
      }
    }
    await page.evaluate(() => { window.endEdit(); window.setCm6EditorEnabled(false); window.setNoteLayoutRtl(false); });
    await page.close();
  }
} finally {
  await browser?.close();
  try { process.kill(-server.pid); } catch {}
}
if (failures.length) { console.error(failures.join("\n")); process.exit(1); }
console.log(`rtl-notes: ok, screenshots in ${out}`);

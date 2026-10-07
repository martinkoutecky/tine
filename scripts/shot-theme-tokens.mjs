// Theming acceptance shots (GH #610 tokens, GH #649 Soft palette).
//   node scripts/shot-theme-tokens.mjs capture <distDir> <outDir>
//       light+dark shots and computed-style metrics of the default rendering
//       (run once on the pre-change build, once on the current one);
//   node scripts/shot-theme-tokens.mjs compare <outDirA> <outDirB>
//       fails unless every screenshot is byte-identical and the metrics equal
//       ("defaults reproduce today's rendering exactly");
//   node scripts/shot-theme-tokens.mjs tokens <distDir> <outDir>
//       sets every public --tine-* token from a graph's custom.css and asserts
//       each one reached its element;
//   node scripts/shot-theme-tokens.mjs soft <distDir> <outDir>
//       the Soft gallery palette, light and dark;
//   node scripts/shot-theme-tokens.mjs overrides <distDir> <outDir>
//       a token set from `html { }` (lower specificity than :root) still wins;
//       the public font tokens win under the editorial typography preset; every
//       monospace surface follows --tine-mono-font.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, readdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { resolve } from "node:path";

const [, , command, a, b] = process.argv;
// A fixed port let a stale `vite preview` of another build answer; pick a fresh one.
const PORT = Number(process.env.SHOT_PORT) || 20000 + Math.floor(Math.random() * 20000);

if (command === "compare") {
  let bad = 0;
  for (const name of readdirSync(a).sort()) {
    const left = readFileSync(resolve(a, name));
    const right = readFileSync(resolve(b, name));
    const same = left.equals(right);
    if (!same) bad++;
    console.log(same ? "IDENTICAL" : "DIFFERENT", name);
  }
  if (bad) { console.error(`${bad} file(s) differ`); process.exit(1); }
  console.log("all identical");
  process.exit(0);
}

const dist = resolve(a);
const out = resolve(b);
mkdirSync(out, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--outDir", dist, "--port", String(PORT), "--strictPort"], { stdio: "ignore", detached: true });
const stop = () => { try { process.kill(-server.pid, "SIGTERM"); } catch {} };

async function waitForServer(url) {
  for (let i = 0; i < 80; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

async function open(browser, customCss = "") {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 1 });
  if (customCss) await context.addInitScript((css) => { globalThis.__tineMockCustomCss = css; }, customCss);
  const page = await context.newPage();
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
  await page.goto(`http://localhost:${PORT}/?regressions`);
  await page.waitForSelector(".page-title", { timeout: 10000 });
  await sleep(500);
  return { context, page };
}

async function mode(page, value, theme = "") {
  await page.evaluate(({ value, theme }) => {
    document.documentElement.setAttribute("data-theme", value);
    window.__tineApplyTheme?.(theme);
  }, { value, theme });
  await sleep(200);
}

async function openEmbedPage(page) {
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("Block embed regression");
  await page.locator(".switcher-row").first().click();
  await page.locator(".block-embed-host[data-block-id]").getByText("Embedded grandchild", { exact: true }).waitFor({ timeout: 8000 });
  await sleep(300);
}

const PROBES = {
  body: ["body", ["font-family", "font-size", "color", "background-color"]],
  embed: [".embed-block", ["background-color"]],
  embedAccent: [".embed-block .block-children", ["border-left-color"]],
  bullet: [".bullet", ["background-color", "width", "height"]],
  tag: [".tag", ["color"]],
  title: [".page-title", ["font-size"]],
  inner: [".main-content-inner", ["max-width"]],
  code: [".inline-code, code", ["font-family"]],
  mark: [".mark, mark", ["background-color"]],
};
async function metrics(page) {
  return page.evaluate((probes) => {
    const out = {};
    for (const [name, [selector, props]] of Object.entries(probes)) {
      const el = document.querySelector(selector);
      out[name] = el ? Object.fromEntries(props.map((p) => [p, getComputedStyle(el).getPropertyValue(p)])) : null;
    }
    return out;
  }, PROBES);
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  if (command === "capture") {
    const { context, page } = await open(browser);
    const all = {};
    for (const m of ["light", "dark"]) {
      await mode(page, m);
      await page.screenshot({ path: `${out}/journals-${m}.png` });
      all[`journals-${m}`] = await metrics(page);
    }
    await openEmbedPage(page);
    for (const m of ["light", "dark"]) {
      await mode(page, m);
      await page.screenshot({ path: `${out}/embed-${m}.png` });
      all[`embed-${m}`] = await metrics(page);
    }
    writeFileSync(`${out}/metrics.json`, JSON.stringify(all, null, 2) + "\n");
    await context.close();
    console.log("captured", Object.keys(all).join(", "));
  } else if (command === "tokens") {
    const css = `:root {
  --tine-embed-bg: rgb(255, 230, 230);
  --tine-embed-accent: rgb(200, 0, 0);
  --tine-bullet-color: rgb(0, 150, 0);
  --tine-bullet-size: 10px;
  --tine-tag-color: rgb(10, 20, 200);
  --tine-highlight-bg: rgb(0, 255, 255);
  --tine-content-width: 600px;
  --tine-page-title-size: 40px;
  --tine-font-size: 19px;
  --tine-content-font: Georgia, serif;
  --tine-mono-font: "Courier New", monospace;
}`;
    const { context, page } = await open(browser, css);
    await mode(page, "light");
    const plain = await metrics(page);
    await page.screenshot({ path: `${out}/tokens-journals-light.png` });
    await openEmbedPage(page);
    const got = await metrics(page);
    await page.screenshot({ path: `${out}/tokens-embed-light.png` });
    const want = [
      ["embed.background-color", got.embed?.["background-color"], "rgb(255, 230, 230)"],
      ["embedded root bullet follows --tine-embed-accent", got.bullet?.["background-color"], "rgb(200, 0, 0)"],
      ["bullet.background-color", plain.bullet?.["background-color"], "rgb(0, 150, 0)"],
      ["bullet.width", plain.bullet?.width, "10px"],
      ["tag.color", plain.tag?.color, "rgb(10, 20, 200)"],
      ["inner.max-width", got.inner?.["max-width"], "600px"],
      ["title.font-size", got.title?.["font-size"], "40px"],
      ["body.font-size", got.body?.["font-size"], "19px"],
    ];
    let bad = 0;
    for (const [name, actual, expected] of want) {
      const ok = actual === expected;
      if (!ok) bad++;
      console.log(ok ? "OK  " : "FAIL", name, actual, ok ? "" : `(wanted ${expected})`);
    }
    const fontOk = (got.body?.["font-family"] ?? "").startsWith("Georgia");
    console.log(fontOk ? "OK  " : "FAIL", "body.font-family", got.body?.["font-family"]);
    if (!fontOk) bad++;
    await context.close();
    if (bad) process.exit(1);
  } else if (command === "overrides") {
    // 1. `html { --tine-x }` has lower specificity than the old `:root { --tine-x: initial }`.
    const htmlRule = `html { --tine-bullet-color: rgb(0, 150, 0); --tine-page-title-size: 40px; }`;
    const first = await open(browser, htmlRule);
    await mode(first.page, "light");
    const viaHtml = await metrics(first.page);
    await first.context.close();
    // 2. Fonts under the editorial typography preset, with and without the tokens.
    const fontRule = `:root { --tine-content-font: Georgia, serif; --tine-editable-font: "Courier New", monospace; --tine-mono-font: "Courier New", monospace; --tine-font-size: 15px; }`;
    const probeFonts = (page) => page.evaluate(() => {
      document.documentElement.setAttribute("data-theme-content-typography", "editorial-serif");
      const section = document.querySelector(".page-section");
      const cs = getComputedStyle(section);
      const out = { section: cs.fontFamily, editable: cs.getPropertyValue("--tine-editable-font").trim(), size: cs.fontSize, mono: {} };
      for (const cls of ["org-timestamp", "journal-conflict-content", "export-preview", "formula-editor-textarea", "today-task-summary"]) {
        const el = document.createElement("span");
        el.className = cls;
        section.appendChild(el);
        out.mono[cls] = getComputedStyle(el).fontFamily;
        el.remove();
      }
      return out;
    });
    const withTokens = await open(browser, fontRule);
    await mode(withTokens.page, "light");
    const fonts = await probeFonts(withTokens.page);
    await withTokens.context.close();
    const plain = await open(browser);
    await mode(plain.page, "light");
    const preset = await probeFonts(plain.page);
    await plain.context.close();
    const checks = [
      ["html{} sets the bullet color", viaHtml.bullet?.["background-color"] === "rgb(0, 150, 0)", viaHtml.bullet?.["background-color"]],
      ["html{} sets the page title size", viaHtml.title?.["font-size"] === "40px", viaHtml.title?.["font-size"]],
      ["preset content font follows --tine-content-font", fonts.section.startsWith("Georgia"), fonts.section],
      ["preset editable font follows --tine-editable-font", fonts.editable.startsWith('"Courier New"'), fonts.editable],
      ["preset font size follows --tine-font-size", fonts.size === "15px", fonts.size],
      ["preset alone still draws the serif face", preset.section.startsWith('"Iowan Old Style"'), preset.section],
      ["preset alone keeps 19px", preset.size === "19px", preset.size],
      ["preset alone keeps its serif editable face", preset.editable.startsWith('"Iowan Old Style"'), preset.editable],
      ...Object.entries(fonts.mono)
        .filter(([cls]) => cls !== "today-task-summary")
        .map(([cls, family]) => [`.${cls} follows --tine-mono-font`, family.startsWith('"Courier New"'), family]),
    ];
    // today-task-summary is body text, not monospace: it follows the content font.
    checks.push(["today-task-summary follows --tine-content-font", fonts.mono["today-task-summary"].startsWith("Georgia"), fonts.mono["today-task-summary"]]);
    let bad = 0;
    for (const [name, ok, actual] of checks) {
      if (!ok) bad++;
      console.log(ok ? "OK  " : "FAIL", name, ok ? "" : `(got ${actual})`);
    }
    if (bad) process.exit(1);
  } else if (command === "soft") {
    const { context, page } = await open(browser);
    for (const m of ["light", "dark"]) {
      await mode(page, m, "soft");
      await page.screenshot({ path: `${out}/soft-${m}.png` });
      const got = await metrics(page);
      console.log("soft", m, JSON.stringify(got.body));
    }
    await context.close();
    if (process.env.SOFT_THUMBNAIL) {
      // Same framing as scripts/shot-theme-gallery.mjs: 640x360 crop at 2x.
      const wide = await browser.newContext({ viewport: { width: 1280, height: 960 }, deviceScaleFactor: 2 });
      const shot = await wide.newPage();
      await shot.goto(`http://localhost:${PORT}/`);
      await shot.waitForSelector(".page-title", { timeout: 10000 });
      await sleep(500);
      await mode(shot, "light", "soft");
      await shot.screenshot({ path: process.env.SOFT_THUMBNAIL, clip: { x: 246, y: 58, width: 640, height: 360 } });
      await wide.close();
    }
  } else {
    throw new Error(`unknown command ${command}`);
  }
  await browser.close();
} finally {
  stop();
}

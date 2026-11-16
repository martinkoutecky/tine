// Theme typography acceptance (GH #760, theme API 0.3), in a real browser engine.
//   node scripts/shot-theme-typography.mjs <distDir> <outDir>
// Drives the literal user path against the built frontend and the mock backend:
// Settings > Appearance > Choose theme.json (a real file input), then the
// installed row's "Use typography", then the Typography selector back to Default.
// For each of 1x, 2x and a narrow phone width it asserts, and photographs:
//   - the declared faces are the ones the engine PAINTS (CDP getPlatformFontsForNode) for
//     block text, inline code and the page title, and the block editor's computed
//     font-family is the declared stack (a textarea has no text node for CDP to report);
//   - bullets stay on the first line centre of their block (the GH #459 oracle);
//   - entering the editor does not move the block (same height, same first line);
//   - a font the device lacks is reported on the row, one it has is not;
//   - reverting restores today's computed fonts and line height exactly, and
//     leaves #tine-theme empty.
// Fonts are chosen from what the host has installed (fc-list); override with
// THEME_TYPO_BODY / THEME_TYPO_MONO / THEME_TYPO_TITLE (a single family name each).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { resolve } from "node:path";

const [, , distArg = "dist", outArg = "artifacts/theme-typo"] = process.argv;
const dist = resolve(distArg);
const out = resolve(outArg);
mkdirSync(out, { recursive: true });
const PORT = Number(process.env.SHOT_PORT) || 20000 + Math.floor(Math.random() * 20000);
const BODY = process.env.THEME_TYPO_BODY ?? "URW Bookman";
const MONO = process.env.THEME_TYPO_MONO ?? "Nimbus Mono PS";
const TITLE = process.env.THEME_TYPO_TITLE ?? "DejaVu Serif";
const LINE_HEIGHT = 34;
const MISSING = "Zzyzx Imaginary Sans";

const base = {
  schemaVersion: 1, version: "1.0.0", author: "Tine", license: "MIT", source: "https://example.invalid/theme",
  apiVersion: "0.3", modes: { light: { "--ls-active-primary-color": "#315efb" }, dark: { "--ls-active-primary-color": "#86a5ff" } }, screenshots: [],
};
const PRESENT = { ...base, id: "page.tine.theme.shot-typography", name: "Shot typography", description: "Fonts this machine has.",
  typography: { bodyFont: `"${BODY}", serif`, monoFont: `"${MONO}", monospace`, titleFont: `"${TITLE}", serif`, blockLineHeight: LINE_HEIGHT } };
const ABSENT = { ...base, id: "page.tine.theme.shot-missing", name: "Shot missing font", description: "A font this machine lacks.",
  typography: { bodyFont: `"${MISSING}", sans-serif` } };

// The editorial-serif preset scopes its own serif editable face and a 30px line on
// `.page-section` (themePresentation.css); a package's typography must win over both,
// through --tine-editable-font-user and --tine-typography-line-height.
const EDITORIAL = { ...base, id: "page.tine.theme.shot-editorial", name: "Shot editorial", description: "Editorial preset plus typography.",
  presentation: { contentTypography: "editorial-serif", journalHeader: "editorial" },
  typography: PRESENT.typography };

const problems = [];
const note = (message) => { problems.push(message); console.log("PROBLEM:", message); };
const server = spawn("npx", ["vite", "preview", "--outDir", dist, "--port", String(PORT), "--strictPort"], { stdio: "ignore", detached: true });
const stop = () => { try { process.kill(-server.pid, "SIGTERM"); } catch {} };

async function waitForServer(url) {
  for (let i = 0; i < 80; i++) { try { if ((await fetch(url)).ok) return; } catch {} await sleep(250); }
  throw new Error("server did not start");
}

/** In-page: bullet centre minus first-line centre for every plain block (GH #459 oracle). */
const measureBullets = () => {
  const rows = [];
  for (const block of document.querySelectorAll(".ls-block")) {
    const main = block.querySelector(":scope > .block-main");
    const content = main?.querySelector(":scope > .block-content-wrapper > .block-content");
    const dot = main?.querySelector(":scope > .block-controls .bullet, :scope > .block-controls .bullet-order");
    if (!content || !dot || content.classList.contains("macro-host")) continue;
    const first = [...content.children].find((child) => !child.classList.contains("block-refs-count")) ?? null;
    if (first && getComputedStyle(first).display !== "inline") continue;
    const style = getComputedStyle(content);
    let lineHeight = parseFloat(style.lineHeight);
    if (first?.classList.contains("heading-text")) lineHeight = Math.max(lineHeight, parseFloat(getComputedStyle(first).lineHeight));
    if (!Number.isFinite(lineHeight)) continue;
    const box = content.getBoundingClientRect();
    const dotBox = dot.getBoundingClientRect();
    const heading = /\bbullet-h[1-6]\b/.exec(main.className)?.[0] ?? null;
    rows.push({ heading, text: (content.textContent ?? "").trim().slice(0, 30), delta: (dotBox.top + dotBox.height / 2) - (box.top + parseFloat(style.paddingTop) + lineHeight / 2) });
  }
  return rows;
};

const computed = () => {
  const q = (selector, props) => {
    const el = document.querySelector(selector);
    return el ? Object.fromEntries(props.map((p) => [p, getComputedStyle(el).getPropertyValue(p)])) : null;
  };
  return {
    body: q("body", ["font-family"]),
    content: q(".block-content", ["font-family", "line-height", "min-height"]),
    code: q(".inline-code, code", ["font-family"]),
    title: q(".page-title", ["font-family"]),
    heading: q(".heading-text", ["font-family"]),
    token: getComputedStyle(document.documentElement).getPropertyValue("--ls-block-line-height").trim(),
    themeCss: document.getElementById("tine-theme")?.textContent ?? "",
  };
};

/** The family the engine actually painted a node's text with. */
async function paintedFamilies(cdp, page, selector) {
  // CDP reports faces for an element's DIRECT text nodes: tag the first element
  // under `selector` that has one.
  await page.evaluate((sel) => {
    document.querySelectorAll("[data-probe]").forEach((el) => el.removeAttribute("data-probe"));
    for (const host of document.querySelectorAll(sel)) {
      const walker = document.createTreeWalker(host, NodeFilter.SHOW_TEXT);
      for (let n = walker.nextNode(); n; n = walker.nextNode()) {
        if (n.textContent.trim().length >= 3) { n.parentElement.setAttribute("data-probe", ""); return; }
      }
    }
  }, selector);
  const { root } = await cdp.send("DOM.getDocument", { depth: 0 });
  const { nodeId } = await cdp.send("DOM.querySelector", { nodeId: root.nodeId, selector: "[data-probe]" });
  if (!nodeId) return null;
  const { fonts } = await cdp.send("CSS.getPlatformFontsForNode", { nodeId });
  return fonts.map((font) => font.familyName);
}

async function openSettingsAppearance(page) {
  await page.locator('button.icon-btn[title^="Settings"]').first().click();
  await page.locator(".settings-modal").waitFor();
  await page.locator(".settings-nav-item", { hasText: "Appearance" }).click();
  await sleep(250);
}

async function installThemeFile(page, manifest) {
  await page.locator('input[type="file"][accept*="json"]').setInputFiles({
    name: "theme.json", mimeType: "application/json", buffer: Buffer.from(JSON.stringify(manifest)),
  });
  await page.locator(".installed-theme-row", { hasText: manifest.name }).waitFor({ timeout: 5000 });
}

async function dismissToasts(page) {
  for (let i = 0; i < 6; i++) {
    const close = page.locator(".toast button[aria-label], .toast-close, .toast .close").first();
    if (!(await close.count())) break;
    await close.click({ timeout: 1000 }).catch(() => {});
  }
  await page.evaluate(() => document.querySelectorAll(".toast, .toasts, .toast-stack").forEach((el) => { el.style.display = "none"; }));
}

const row = (page, name) => page.locator(".installed-theme-row", { hasText: name });

async function run(browser, label, viewport, deviceScaleFactor) {
  const context = await browser.newContext({ viewport, deviceScaleFactor });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e).split("\n")[0]));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 10000 });
  await sleep(500);
  const cdp = await context.newCDPSession(page);
  await cdp.send("DOM.enable");
  await cdp.send("CSS.enable");

  const before = await page.evaluate(computed);
  const beforeBullets = await page.evaluate(measureBullets);
  if (before.themeCss !== "") note(`${label}: #tine-theme is not empty before any selection`);

  await openSettingsAppearance(page);
  await installThemeFile(page, PRESENT);
  await installThemeFile(page, ABSENT);
  const absentHints = await row(page, ABSENT.name).locator(".installed-theme-font-hint").allTextContents();
  const presentHints = await row(page, PRESENT.name).locator(".installed-theme-font-hint").count();
  if (absentHints.length !== 1 || !absentHints[0].includes(MISSING)) note(`${label}: expected one not-installed hint naming ${MISSING}, saw ${JSON.stringify(absentHints)}`);
  if (presentHints !== 0) note(`${label}: a font this machine has was reported as not installed`);
  if (await row(page, ABSENT.name).locator("button", { hasText: "Use typography" }).isDisabled()) note(`${label}: a missing font blocked Use typography`);
  await page.locator(".installed-theme-list").scrollIntoViewIfNeeded();
  await page.locator(".installed-theme-list").screenshot({ path: `${out}/settings-${label}.png` });

  await row(page, PRESENT.name).locator("button", { hasText: "Use typography" }).click();
  await sleep(400);
  const during = await page.evaluate(computed);
  if (!during.content?.["font-family"]?.includes(BODY)) note(`${label}: block text did not take the body font (${during.content?.["font-family"]})`);
  if (!during.code?.["font-family"]?.includes(MONO)) note(`${label}: inline code did not take the mono font (${during.code?.["font-family"]})`);
  if (!during.title?.["font-family"]?.includes(TITLE)) note(`${label}: the page title did not take the title font (${during.title?.["font-family"]})`);
  if (during.token !== `${LINE_HEIGHT}px`) note(`${label}: --ls-block-line-height is ${during.token}, expected ${LINE_HEIGHT}px`);
  if (during.content?.["line-height"] !== `${LINE_HEIGHT}px`) note(`${label}: block line-height is ${during.content?.["line-height"]}`);

  // The engine's own answer to "which face was drawn".
  await page.keyboard.press("Escape");
  await sleep(300);
  if (await page.locator(".settings-modal").count()) await page.locator(".settings-modal .close, .settings-close").first().click().catch(() => {});
  const painted = {
    block: await paintedFamilies(cdp, page, ".block-content"),
    code: await paintedFamilies(cdp, page, ".inline-code, code"),
    title: await paintedFamilies(cdp, page, ".page-title"),
  };
  if (!painted.block?.some((name) => name.includes(BODY))) note(`${label}: block text painted with ${JSON.stringify(painted.block)}, not ${BODY}`);
  if (!painted.code?.some((name) => name.includes(MONO))) note(`${label}: inline code painted with ${JSON.stringify(painted.code)}, not ${MONO}`);
  if (!painted.title?.some((name) => name.includes(TITLE))) note(`${label}: the title painted with ${JSON.stringify(painted.title)}, not ${TITLE}`);

  const rows = await page.evaluate(measureBullets);
  if (rows.length < 5) note(`${label}: only ${rows.length} measurable blocks`);
  for (const r of rows) {
    const limit = r.heading ? 3 : 0.5;
    if (Math.abs(r.delta) > limit) note(`${label}: bullet ${r.delta.toFixed(2)}px off the first line centre on ${JSON.stringify(r.text)}`);
  }
  await dismissToasts(page);
  await page.screenshot({ path: `${out}/page-${label}.png` });

  // Entering the editor must not move the block: same top, same height, the
  // textarea painted with the declared face.
  const target = page.locator(".block-content").filter({ hasText: /\w{4}/ }).nth(2);
  const staticBox = await target.boundingBox();
  await target.click({ position: { x: 20, y: 8 } });
  await page.locator("textarea.block-editor").first().waitFor({ timeout: 4000 });
  await sleep(250);
  const editor = page.locator("textarea.block-editor").first();
  const editorBox = await editor.boundingBox();
  const editorFamily = await editor.evaluate((el) => getComputedStyle(el).fontFamily);
  if (!editorFamily.includes(BODY)) note(`${label}: the editor font-family is ${editorFamily}`);
  if (staticBox && editorBox && Math.abs(staticBox.height - editorBox.height) > 1.5) {
    note(`${label}: entering the editor changed the block height ${staticBox.height} -> ${editorBox.height}`);
  }
  if (staticBox && editorBox && Math.abs(staticBox.y - editorBox.y) > 1.5) note(`${label}: entering the editor moved the block ${staticBox.y} -> ${editorBox.y}`);
  await dismissToasts(page);
  await editor.scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${out}/editing-${label}.png` });
  await page.keyboard.press("Escape");
  await sleep(200);

  // Revert through the selector: today's rendering comes back exactly.
  await openSettingsAppearance(page);
  await page.locator('select[aria-label="Typography"]').selectOption("");
  await sleep(400);
  const after = await page.evaluate(computed);
  for (const key of Object.keys(before)) {
    if (JSON.stringify(before[key]) !== JSON.stringify(after[key])) note(`${label}: reverting did not restore ${key}: ${JSON.stringify(before[key])} -> ${JSON.stringify(after[key])}`);
  }
  await page.keyboard.press("Escape");
  await sleep(200);
  const afterBullets = await page.evaluate(measureBullets);
  if (JSON.stringify(beforeBullets.map((r) => +r.delta.toFixed(2))) !== JSON.stringify(afterBullets.map((r) => +r.delta.toFixed(2)))) note(`${label}: bullet alignment differs after revert`);
  if (errors.length) note(`${label}: page errors ${JSON.stringify(errors)}`);
  console.log(label, "painted:", JSON.stringify(painted), "editor:", editorFamily.slice(0, 60), "bullets:", rows.length, "blocks, max |delta|", Math.max(...rows.map((r) => Math.abs(r.delta))).toFixed(2));
  await context.close();
}

async function runEditorial(browser, label, viewport, deviceScaleFactor) {
  const context = await browser.newContext({ viewport, deviceScaleFactor });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e).split("\n")[0]));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 10000 });
  await sleep(500);
  const cdp = await context.newCDPSession(page);
  await cdp.send("DOM.enable");
  await cdp.send("CSS.enable");

  await openSettingsAppearance(page);
  await installThemeFile(page, EDITORIAL);
  await row(page, EDITORIAL.name).locator("button", { hasText: "Use style" }).click();
  await sleep(400);
  const styled = await page.evaluate(computed);
  // The preset sets its 30px on .page-section, not :root: read the block, not the root token.
  if (styled.content?.["line-height"] !== "30px") note(`${label}/editorial: the preset alone should give a 30px line, saw ${styled.content?.["line-height"]}`);
  if (/Bookman/.test(styled.content?.["font-family"] ?? "")) note(`${label}/editorial: the preset alone already shows the package font`);
  await row(page, EDITORIAL.name).locator("button", { hasText: "Use typography" }).click();
  await sleep(400);
  const during = await page.evaluate(computed);
  if (during.content?.["line-height"] !== `${LINE_HEIGHT}px`) note(`${label}/editorial: block line-height is ${during.content?.["line-height"]}`);
  if (!during.content?.["font-family"]?.includes(BODY)) note(`${label}/editorial: block text font is ${during.content?.["font-family"]}`);
  await page.keyboard.press("Escape");
  await sleep(300);
  const painted = { block: await paintedFamilies(cdp, page, ".block-content"), title: await paintedFamilies(cdp, page, ".journal-title, .page-title") };
  if (!painted.block?.some((name) => name.includes(BODY))) note(`${label}/editorial: block text painted with ${JSON.stringify(painted.block)}, not ${BODY}`);
  if (!painted.title?.some((name) => name.includes(TITLE))) note(`${label}/editorial: title painted with ${JSON.stringify(painted.title)}, not ${TITLE}`);
  const rows = await page.evaluate(measureBullets);
  if (rows.length < 5) note(`${label}/editorial: only ${rows.length} measurable blocks`);
  for (const r of rows) {
    if (Math.abs(r.delta) > (r.heading ? 3 : 0.5)) note(`${label}/editorial: bullet ${r.delta.toFixed(2)}px off the first line centre on ${JSON.stringify(r.text)}`);
  }
  await dismissToasts(page);
  await page.screenshot({ path: `${out}/editorial-page-${label}.png` });

  const target = page.locator(".block-content").filter({ hasText: /\w{4}/ }).nth(2);
  const staticBox = await target.boundingBox();
  await target.click({ position: { x: 20, y: 8 } });
  await page.locator("textarea.block-editor").first().waitFor({ timeout: 4000 });
  await sleep(250);
  const editor = page.locator("textarea.block-editor").first();
  const editorBox = await editor.boundingBox();
  const editorFamily = await editor.evaluate((el) => getComputedStyle(el).fontFamily);
  if (!editorFamily.includes(BODY)) note(`${label}/editorial: the editor font-family is ${editorFamily} (the preset's serif won over the package)`);
  if (/Iowan/.test(editorFamily)) note(`${label}/editorial: the editor still uses the preset's Iowan stack`);
  if (staticBox && editorBox && Math.abs(staticBox.height - editorBox.height) > 1.5) note(`${label}/editorial: entering the editor changed the block height ${staticBox.height} -> ${editorBox.height}`);
  if (staticBox && editorBox && Math.abs(staticBox.y - editorBox.y) > 1.5) note(`${label}/editorial: entering the editor moved the block ${staticBox.y} -> ${editorBox.y}`);
  await dismissToasts(page);
  await editor.scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${out}/editorial-editing-${label}.png` });
  await page.keyboard.press("Escape");
  await sleep(200);

  // Dropping only the typography returns the preset's own face and 30px line.
  await openSettingsAppearance(page);
  await page.locator('select[aria-label="Typography"]').selectOption("");
  await sleep(400);
  const after = await page.evaluate(computed);
  if (after.content?.["line-height"] !== "30px") note(`${label}/editorial: after dropping typography the line is ${after.content?.["line-height"]}, expected the preset's 30px`);
  if (after.content?.["font-family"] !== styled.content?.["font-family"]) note(`${label}/editorial: dropping typography did not restore the preset's block face`);
  if (errors.length) note(`${label}/editorial: page errors ${JSON.stringify(errors)}`);
  console.log(label, "editorial painted:", JSON.stringify(painted), "editor:", editorFamily.slice(0, 60), "max |delta|", Math.max(...rows.map((r) => Math.abs(r.delta))).toFixed(2));
  await context.close();
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  await run(browser, "1x", { width: 1280, height: 900 }, 1);
  await run(browser, "2x", { width: 1280, height: 900 }, 2);
  await run(browser, "narrow", { width: 390, height: 844 }, 2);
  await runEditorial(browser, "1x", { width: 1280, height: 900 }, 1);
  await runEditorial(browser, "2x", { width: 1280, height: 900 }, 2);
  await browser.close();
} finally {
  stop();
}
if (problems.length) {
  console.error(`${problems.length} problem(s)`);
  process.exit(1);
}
console.log("theme typography: all checks passed; shots in", out);

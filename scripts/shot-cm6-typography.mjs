// a block with arrows and dashes looks the same before and after clicking into it (CodeMirror editor,
// default typography setting), and shows the typed characters only where the caret is. Routed page, Chromium.
// usage: node scripts/shot-cm6-typography.mjs   (screenshots in target/artifacts/cm6-typography/)
import assert from "node:assert/strict";
import { mkdirSync, realpathSync } from "node:fs";
import { chromium } from "playwright";
import { createServer } from "vite";
import solid from "vite-plugin-solid";

const out = "target/artifacts/cm6-typography";
mkdirSync(out, { recursive: true });
const TEXT = "Arrows -> and dashes -- and --- with `code -- kept`";
const server = await createServer({ configFile: false, plugins: [solid()],
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("cm6-typography"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, fs: { allow: [process.cwd(), realpathSync("node_modules"), realpathSync("node_modules/inter-ui")] } } });
await server.listen();
let browser;
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  page.setDefaultTimeout(30_000);
  await page.goto(server.resolvedUrls.local[0], { waitUntil: "commit" });
  await page.locator("[data-search-trigger]").waitFor();
  for (const close of await page.locator(".toast-close").all()) await close.click();
  await page.evaluate(async () => (await import("/src/router.ts")).openPage("Tine"));
  await page.locator(".page-title").filter({ hasText: "Tine" }).first().waitFor();
  // Write the sample into the first block through the editor, as a user would.
  await page.locator(".main-content .block-content-wrapper").first().click();
  const content = page.locator('.main-content [data-block-editor="cm6"] .cm-content');
  await content.waitFor();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type(TEXT);
  await page.keyboard.press("Escape");
  const row = page.locator(".main-content .ls-block").first();
  const rendered = page.locator(".main-content .block-content").first();
  await rendered.waitFor();
  const readText = (await rendered.innerText()).trim();
  assert.equal(readText, "Arrows → and dashes – and — with code -- kept", "the view draws the glyphs (default setting) and leaves code raw");
  await row.screenshot({ path: `${out}/1-rendered.png` });
  // Click at the end of the block: the caret is away from every replaced sequence.
  const box = await rendered.boundingBox();
  await page.mouse.click(box.x + box.width - 2, box.y + box.height / 2);
  await content.waitFor();
  await page.keyboard.press("End");
  const editText = (await content.innerText()).trim();
  assert.equal(editText, "Arrows → and dashes – and — with `code -- kept`", "the editor shows the same glyphs, code markers revealed only at the caret");
  await row.screenshot({ path: `${out}/2-editing-caret-at-end.png` });
  // Move the caret into `--`: that sequence alone shows its typed characters.
  await page.keyboard.press("Home");
  for (let i = 0; i < "Arrows -> and dashes -".length; i++) await page.keyboard.press("ArrowRight");
  // The code span, now away from the caret, is drawn as the view draws it.
  assert.equal((await content.innerText()).trim(), "Arrows → and dashes -- and — with code -- kept");
  await row.screenshot({ path: `${out}/3-editing-caret-in-dashes.png` });
  console.log(`PASS: rendered "${readText}"; editing "${editText}"; screenshots in ${out}`);
  await page.close();
} finally { await browser?.close(); await server.close(); }

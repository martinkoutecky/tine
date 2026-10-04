// Chromium exercises the real Block/AST renderer and pane CSS, not jsdom layout.
import { chromium } from 'playwright';
import { build, preview } from 'vite';
import solid from 'vite-plugin-solid';
import { setTimeout as sleep } from 'node:timers/promises';
import fs from 'node:fs';
const port = Number(process.env.QBG_PORT || 5319);
const origin = `http://127.0.0.1:${port}`;
const out = process.env.QBG_OUT || '/tmp/qbg-after';
fs.mkdirSync(out, { recursive: true });
const buildDir = `/tmp/qbg-shot-build-${process.pid}`;
await build({ configFile: false, define: { __BUILD_TIME__: JSON.stringify('fixture'), __GIT_COMMIT__: JSON.stringify('fixture'), __TINE_COMMUNITY_REGISTRY__: 'false' }, plugins: [solid()], build: { outDir: buildDir, copyPublicDir: false, rollupOptions: { input: 'scripts/fixtures/table-bleed.html' } } });
const server = await preview({ configFile: false, build: { outDir: buildDir }, preview: { host: '127.0.0.1', port, strictPort: true } });
let browser;
const failures = [];
try {
  browser = await chromium.launch({ args: ['--no-sandbox'] });
  const cases = [
    ['narrow', 1600, 1400, 0, 8], ['wide', 1600, 1400, 0, 65],
    ['overflow', 1600, 1400, 0, 130], ['nested', 1600, 1400, 3, 65],
    ['split', 1400, 680, 3, 65], ['mobile', 390, 390, 3, 65],
    ['org', 1600, 1400, 3, 65], ['query', 1600, 1400, 0, 65],
    ['sheet', 1600, 1400, 0, 65], ['sidebar', 600, 500, 3, 65], ['embed', 1600, 1400, 3, 65],
  ];
  for (const [name, viewport, pane, depth, length] of cases) {
    const page = await browser.newPage({ viewport: { width: viewport, height: 700 } });
    page.on("pageerror", error => console.error("PAGE ERROR", error));
    page.setDefaultTimeout(120000);
    await page.goto(`${origin}/scripts/fixtures/table-bleed.html`, { waitUntil: 'domcontentloaded', timeout: 120000 });
    const raw = `| First | Second |\n${name === 'org' ? '|-------+--------|' : '| --- | --- |'}\n| ${'a'.repeat(length)} | ${'b'.repeat(length)} |`;
    await page.evaluate(async ({ raw, depth, pane, format, surface }) => { await window.qbgMount(raw, depth, pane, format, surface); }, { raw, depth, pane, format: name === 'org' ? 'org' : 'md', surface: name });
    await page.locator(name === 'sheet' ? '.sheet-table' : '.md-table').waitFor();
    await sleep(600);
    const geometry = await page.evaluate(() => {
      const wrap = document.querySelector('.md-table-wrap, .sheet-scroll');
      const table = wrap.querySelector('table, .sheet-table');
      const pane = wrap.closest('.rs-item-body, .right-sidebar-body, .main-content');
      const column = wrap.closest('.main-content-inner');
      const r = wrap.getBoundingClientRect(), p = pane.getBoundingClientRect(), c = column.getBoundingClientRect();
      return { left: r.left, right: r.right, width: r.width, paneLeft: p.left, paneRight: p.left + pane.clientWidth, center: (c.left + c.right) / 2, scroll: wrap.scrollWidth - wrap.clientWidth, tableWidth: table.getBoundingClientRect().width, paneScroll: pane.scrollWidth - pane.clientWidth };
    });
    await page.screenshot({ path: `${out}/${name}.png` });
    if (name === 'narrow') {
      const oldLayout = await page.addStyleTag({ content: '.md-table-wrap { width:100%; margin:1rem 0; } .md-table-wrap > .md-table { width:100%; min-width:0; }' });
      const original = await page.locator('.md-table-wrap').boundingBox();
      if (!original || Math.abs(original.x - geometry.left) > 1 || Math.abs(original.width - geometry.width) > 1) failures.push('narrow: changed fitting table layout');
      await oldLayout.evaluate(el => el.remove());
    }
    console.log(name, geometry);
    if (geometry.left < geometry.paneLeft - 1 || geometry.right > geometry.paneRight + 1 || geometry.paneScroll > 1) failures.push(`${name}: escaped pane`);
    if (['wide', 'nested', 'org', 'query', 'sheet', 'embed'].includes(name) && (geometry.scroll > 1 || Math.abs((geometry.left + geometry.right) / 2 - geometry.center) > 2)) failures.push(`${name}: not centered and freely bleeding`);
    if (['overflow', 'split', 'mobile', 'sidebar'].includes(name) && geometry.scroll < 1) failures.push(`${name}: no horizontal scrolling`);
    if (name === 'sheet') {
      const before = await page.locator('.sheet-scroll').boundingBox();
      await page.locator('.sheet-field-cell').filter({ hasText: /editable/ }).dblclick();
      const editor = page.locator('.sheet-prop-input').first();
      await editor.waitFor();
      await editor.fill('a longer edit that stays within its fixed cell');
      await sleep(250);
      const after = await page.locator('.sheet-scroll').boundingBox();
      if (!before || !after || Math.abs(before.x - after.x) > 1 || Math.abs(before.width - after.width) > 1) failures.push('sheet: layout jumps while typing');
      await page.screenshot({ path: `${out}/sheet-edit.png` });
    }
    await page.close();
  }
} finally { await browser?.close(); await new Promise(resolve => server.httpServer.close(resolve)); }
fs.writeFileSync(`${out}/result.json`, JSON.stringify(failures));
if (failures.length) { console.error(failures.join('\n')); process.exitCode = 1; }

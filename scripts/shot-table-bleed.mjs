// Chromium exercises the real Block/AST renderer and pane CSS, not jsdom layout.
import { chromium } from 'playwright';
import { build, preview } from 'vite';
import solid from 'vite-plugin-solid';
import { setTimeout as sleep } from 'node:timers/promises';
import fs from 'node:fs';
const port = Number(process.env.QBG_PORT || 5319);
const origin = `http://127.0.0.1:${port}`;
const out = process.env.QBG_OUT || '/tmp/qbg-after';
// The query-box cases (GH #651 follow-up, D-20261028-UILP): a result table never leaves its query box.
const words = Array.from({ length: 30 }, (_, i) => `word${i}`).join(' ');
fs.mkdirSync(out, { recursive: true });
const buildDir = `${out}/build-${process.pid}`;
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
    ['sheet', 1600, 1400, 0, 65], ['query-words', 1600, 1400, 0, 0, words], ['query-words-narrow', 1000, 560, 0, 0, words], ['sidebar', 600, 500, 3, 65], ['embed', 1600, 1400, 3, 65],
    // GH #802: prose of every length wraps only after using available pane space.
    // Intentional OG difference: preserve fitting tables and expand beyond the
    // text column before wrapping, rather than always filling the column.
    ['words', 1600, 1400, 0, 0, words], ['words-mobile', 390, 390, 0, 0, words],
    ['words-sidebar', 600, 500, 0, 0, words], ['words-embed', 1000, 560, 0, 0, words],
    // A short prose column beside a long one keeps its words whole on desktop:
    // last-resort word splitting must not squeeze it while the table fits.
    ['mixed', 1000, 560, 0, 0, words],
    ['many-columns', 1000, 560, 0, 0, 'short cell words'],
    ['many-columns-mobile', 390, 390, 0, 0, 'short cell words'],
    ['long-url', 390, 390, 0, 0, `https://example.test/${'segment/'.repeat(24)}`],
    ['reported', 390, 390, 0, 0],
  ];
  for (const [name, viewport, pane, depth, length, cell] of cases) {
    const page = await browser.newPage({ viewport: { width: viewport, height: 700 } });
    page.on("pageerror", error => console.error("PAGE ERROR", error));
    page.setDefaultTimeout(120000);
    await page.goto(`${origin}/scripts/fixtures/table-bleed.html`, { waitUntil: 'domcontentloaded', timeout: 120000 });
    const count = name.startsWith('many-columns') ? 7 : 2;
    const row = value => `| ${Array.from({ length: count }, (_, i) => value(i)).join(' | ')} |`;
    // The literal issue snippet has a very long separator, not a data cell.
    const raw = name === 'reported' ? `| - | - |- |\n|---|${'-'.repeat(402)}|-----------|`
      : `${row(i => `Col ${i}`)}\n${name === 'org' ? '|-------+--------|' : row(() => '---')}\n${row(i => name === 'mixed' ? (i === 0 ? 'Status column words' : `${words} ${words} ${words} ${words}`) : cell ?? (i ? 'b' : 'a').repeat(length))}`;
    const surface = name.endsWith('-sidebar') ? 'sidebar' : name.endsWith('-embed') ? 'embed' : name;
    await page.evaluate(async ({ raw, depth, pane, format, surface, cell }) => { await window.qbgMount(raw, depth, pane, format, surface, cell); }, { raw, depth, pane, format: name === 'org' ? 'org' : 'md', surface, cell: cell ?? 'a'.repeat(length) });
    await page.locator(name === 'sheet' ? '.sheet-table' : '.md-table').waitFor();
    await sleep(600);
    const geometry = await page.evaluate(() => {
      const wrap = document.querySelector('.md-table-wrap, .sheet-scroll');
      const table = wrap.querySelector('table, .sheet-table');
      const pane = wrap.closest('.rs-item-body, .right-sidebar-body, .main-content');
      const column = wrap.closest('.main-content-inner');
      const r = wrap.getBoundingClientRect(), p = pane.getBoundingClientRect(), c = column.getBoundingClientRect();
      const b = wrap.closest('.query-block')?.getBoundingClientRect();
      return { boxLeft: b?.left, boxRight: b?.right, left: r.left, right: r.right, width: r.width, paneLeft: p.left, paneRight: p.left + pane.clientWidth, center: (c.left + c.right) / 2, scroll: wrap.scrollWidth - wrap.clientWidth, tableWidth: table.getBoundingClientRect().width, paneScroll: pane.scrollWidth - pane.clientWidth };
    });
    await page.screenshot({ path: `${out}/${name}.png` });
    if (name === 'narrow') {
      const oldLayout = await page.addStyleTag({ content: '.md-table-wrap { width:100%; margin:1rem 0; } .md-table-wrap > .md-table { width:100%; min-width:0; }' });
      const original = await page.locator('.md-table-wrap').boundingBox();
      if (!original || Math.abs(original.x - geometry.left) > 1 || Math.abs(original.width - geometry.width) > 1) failures.push('narrow: changed fitting table layout');
      await oldLayout.evaluate(el => el.remove());
    }
    console.log(name, geometry);
    if (name === 'mixed' || name === 'words') {
      const whole = await page.evaluate(() => {
        const wrap = document.querySelector('.md-table-wrap');
        const td = wrap.querySelector('tbody td, tr:last-child td');
        const style = getComputedStyle(td);
        const probe = document.createElement('span');
        probe.textContent = td.textContent.trim().split(/\s+/).reduce((a, b) => (b.length > a.length ? b : a), '');
        probe.style.cssText = `white-space:nowrap;font:${style.font};position:absolute;visibility:hidden`;
        document.body.append(probe);
        const need = probe.getBoundingClientRect().width + parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
        probe.remove();
        return { cell: td.getBoundingClientRect().width, need, tight: wrap.classList.contains('md-table-tight') };
      });
      console.log(name, 'whole-words', whole);
      if (whole.tight || whole.cell + 0.5 < whole.need) failures.push(`${name}: a fitting table splits a short column's words`);
    }
    if (name.startsWith('words') || name.startsWith('many-columns') || name === 'mixed' || name === 'long-url' || name === 'reported') {
      if (geometry.scroll > 1) failures.push(`${name}: wrapping content scrolls instead of fitting the pane`);
      if (name === 'words' && geometry.width <= 720) failures.push('words: wraps before using free pane space');
    }
    if (geometry.left < geometry.paneLeft - 1 || geometry.right > geometry.paneRight + 1 || geometry.paneScroll > 1) failures.push(`${name}: escaped pane`);
    if (name.startsWith('query')) {
      // A table in a query box is bounded by the box: it never bleeds, and a row wider than the box scrolls inside it.
      if (geometry.left < geometry.boxLeft - 1 || geometry.right > geometry.boxRight + 1) failures.push(`${name}: table leaves its query box`);
      if (name === 'query' && geometry.scroll < 1) failures.push('query: an unbreakable wide row does not scroll inside its box');
      if (name.startsWith('query-words') && geometry.scroll > 1) failures.push(`${name}: text with spaces should wrap inside the box, not scroll`);
    }
    if (['wide', 'nested', 'org', 'sheet', 'embed'].includes(name) && (geometry.scroll > 1 || Math.abs((geometry.left + geometry.right) / 2 - geometry.center) > 2)) failures.push(`${name}: not centered and freely bleeding`);
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
    if (name === 'words-mobile') {
      await page.setViewportSize({ width: 1600, height: 700 });
      await page.evaluate(() => { document.querySelector('.pane-leaf').style.width = '1400px'; });
      await sleep(200);
      const grown = await page.locator('.md-table-wrap').boundingBox();
      if (!grown || grown.width <= geometry.width) failures.push('words resize: table does not reclaim available pane space');
      await page.screenshot({ path: `${out}/words-resize.png` });
    }
    await page.close();
  }
} finally { await browser?.close(); await new Promise(resolve => server.httpServer.close(resolve)); }
fs.writeFileSync(`${out}/result.json`, JSON.stringify(failures));
if (failures.length) { console.error(failures.join('\n')); process.exitCode = 1; }

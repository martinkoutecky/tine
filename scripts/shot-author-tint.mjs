// GH #893: another author's block sits on a faint author-coloured tint behind its own row.
// Real browser app on the in-memory `?regressions` mock graph ("Author mark gap",
// "Author mark copies", "Author tint comment"). Measures and screenshots.
// Run: node scripts/shot-author-tint.mjs [artifactDir]
// Fail-before: on a checkout without the tint there is no `.author-tinted` row and the
// gutter line element still exists, so the first assertion fails.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createServer } from "vite";
import solid from "vite-plugin-solid";
import { chromium } from "playwright";

const out = process.argv[2] ?? "target/artifacts/author-mark-893/tint";
const commit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
mkdirSync(out, { recursive: true });
const server = await createServer({ configFile: false, plugins: [solid()],
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("author-tint"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, watch: { ignored: ["**/target/**", "**/.toolchain/**", "**/.cargo/**"] } },
});
await server.listen();
let browser, activePage;
const results = [], failures = [], browserErrors = [];

/** Per bulleted row in scope: bullet box, tint (the wrapper's own background), first letter. */
function measureRows(scopeEl) {
  const rows = [];
  for (const row of scopeEl.querySelectorAll(".ls-block")) {
    const main = row.querySelector(":scope > .block-main");
    const bullet = main?.querySelector(":scope > .block-controls .bullet-container");
    const wrapper = main?.querySelector(":scope > .block-content-wrapper");
    if (!bullet || !wrapper || !wrapper.offsetParent) continue;
    if (wrapper.querySelector(":scope > .block-content .ls-block")) continue; // embed/query host: its rows are measured themselves
    const editor = wrapper.querySelector("textarea.block-editor");
    const cm = wrapper.querySelector(".cm-content");
    const firstText = root => {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      let node;
      while ((node = walker.nextNode()))
        if (node.textContent.trim() && !node.parentElement.closest(".author-chip, .block-refs-count, .margin-marker, .bullet-tooltip")) return node;
      return null;
    };
    let textX = null;
    if (editor) {
      const s = getComputedStyle(editor);
      textX = editor.getBoundingClientRect().left + parseFloat(s.paddingLeft) + parseFloat(s.borderLeftWidth);
    } else {
      const node = firstText(cm ?? wrapper.querySelector(":scope > .block-content") ?? wrapper);
      if (node) {
        const at = node.textContent.search(/\S/);
        const range = document.createRange(); range.setStart(node, at); range.setEnd(node, at + 1);
        textX = range.getBoundingClientRect().left;
      }
    }
    const bg = getComputedStyle(wrapper).backgroundColor;
    const wr = wrapper.getBoundingClientRect(), br = bullet.getBoundingClientRect();
    const authored = row.classList.contains("authored");
    rows.push({ label: (firstText(wrapper)?.textContent ?? "").trim().slice(0, 32), authored,
      tinted: row.classList.contains("author-tinted"), hasBackground: bg !== "rgba(0, 0, 0, 0)" && bg !== "transparent",
      selected: main.classList.contains("selected"), gutterLines: row.querySelectorAll(":scope > .block-main .author-mark").length,
      folded: bullet.classList.contains("bullet-closed"), ordered: bullet.classList.contains("ordered"), editing: !!(editor || cm),
      continued: row.classList.contains("author-run-continued"), continues: row.classList.contains("author-run-continues"),
      title: wrapper.getAttribute("title"), bulletRight: br.right, textX,
      wrapperLeft: wr.left, wrapperRight: wr.right, top: wr.top, bottom: wr.bottom });
  }
  return rows;
}

const r2 = n => Math.round(n * 100) / 100;
function check(name, rows, { needAuthored = 1, needFolded = false, needEditing = false, switchOff = false } = {}) {
  const authored = rows.filter(x => x.authored), plain = rows.filter(x => !x.authored && !x.ordered);
  results.push({ name, rows: rows.map(x => ({ label: x.label, authored: x.authored, tinted: x.tinted, hasBackground: x.hasBackground, editing: x.editing,
    folded: x.folded, ordered: x.ordered, selected: x.selected, textMinusBullet: x.textX == null ? null : r2(x.textX - x.bulletRight),
    tintLeftOfText: r2(x.textX - x.wrapperLeft), tintRightPast: r2(x.wrapperRight) })) });
  try {
    assert.ok(authored.length >= needAuthored, `${name}: expected ${needAuthored} authored rows, saw ${authored.length}`);
    if (needFolded) assert.ok(authored.some(x => x.folded), `${name}: no folded authored row`);
    if (needEditing) assert.ok(authored.some(x => x.editing), `${name}: no authored row in an editor`);
    for (const x of authored) {
      assert.equal(x.gutterLines, 0, `${name} "${x.label}": a gutter line element is still drawn`);
      if (switchOff) { assert.ok(!x.tinted && !x.hasBackground, `${name} "${x.label}": tint drawn with the switch off`); continue; }
      assert.ok(x.tinted, `${name} "${x.label}": authored row is not tinted`);
      if (x.selected) assert.ok(!x.hasBackground, `${name} "${x.label}": selected row keeps its tint under the selection colour`);
      else assert.ok(x.hasBackground, `${name} "${x.label}": tinted row has no background`);
    }
    for (const x of plain) assert.ok(!x.tinted && !x.hasBackground, `${name} "${x.label}": an own block is tinted`);
    // First letter: the same bullet-to-letter offset whether authored or not (numbered rows have 4px).
    const offsets = [...authored.filter(x => !x.ordered), ...plain].filter(x => x.textX != null).map(x => x.textX - x.bulletRight);
    if (offsets.length > 1) assert.ok(Math.max(...offsets) - Math.min(...offsets) <= 0.5, `${name}: first-letter offset varies ${offsets.map(r2)}`);
    if (!switchOff) for (const x of authored.filter(x => x.tinted && x.textX != null && !x.editing)) {
      const into = x.textX - x.wrapperLeft;
      assert.ok(into >= 2.5 && into <= 4.5, `${name} "${x.label}": tint starts ${r2(into)}px left of the text, expected 3-4px (inside the bullet gap)`);
    }
    // A same-author run is one card: neighbouring tinted rows touch.
    if (!switchOff) for (let i = 0; i + 1 < rows.length; i++) {
      const a = rows[i], b = rows[i + 1];
      if (a.continues && b.continued && a.tinted && b.tinted) assert.ok(Math.abs(a.bottom - b.top) <= 0.5, `${name} "${a.label}"/"${b.label}": run members do not touch (${r2(b.top - a.bottom)}px)`);
    }
  } catch (error) { failures.push(String(error.message ?? error)); }
}

async function newPage(width, theme, dsf = 2, { hidden = false } = {}) {
  const ctx = await browser.newContext({ viewport: { width, height: 900 }, hasTouch: width === 390, deviceScaleFactor: dsf });
  const page = await ctx.newPage();
  page.setDefaultTimeout(15_000);
  page.on("pageerror", error => browserErrors.push(String(error)));
  await page.addInitScript(hide => {
    localStorage.setItem("logseq-claude.sidebarOpen", "0");
    if (hide) localStorage.setItem("logseq-claude.author-marks-hidden", "1");
  }, hidden);
  await page.goto(`${server.resolvedUrls.local[0]}?regressions`);
  await page.locator(".ls-block").first().waitFor();
  await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
  await page.addStyleTag({ content: ".toast-stack { display: none !important; }" });
  const open = async name => {
    await page.keyboard.press("Control+k");
    await page.locator(".switcher-input").fill(name);
    await page.locator(".switcher-row").filter({ hasText: name }).first().click();
    await page.locator(".page-title").filter({ hasText: name }).first().waitFor();
    if (await page.locator(".mobile-drawer-scrim").count()) await page.keyboard.press("Escape");
    await page.evaluate(() => document.fonts.ready);
    await page.mouse.move(0, 0);
  };
  return { ctx, page, open };
}

try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const [width, theme] of [[1280, "light"], [1280, "dark"], [390, "light"], [390, "dark"]]) {
    const { ctx, page, open } = await newPage(width, theme);
    activePage = page;
    const tag = `${width}-${theme}`;
    const measure = (name, scope, opts) => scope.evaluate(measureRows).then(rows => check(`${tag} ${name}`, rows, opts));
    await open("Author mark gap");
    const main = page.locator(".page-blocks").first();
    await main.locator(".ls-block.authored .bullet-closed").first().waitFor();
    await measure("main", main, { needAuthored: 6, needFolded: true });
    await page.screenshot({ path: `${out}/${tag}-page.png` });
    // Live copies: block embed and linked references (the mock has no query engine).
    await open("Author mark copies");
    await page.locator(".embed-block .ls-block.authored").first().waitFor();
    await page.waitForTimeout(600);
    await measure("embed", page.locator(".page-blocks").first(), { needAuthored: 1 });
    await page.screenshot({ path: `${out}/${tag}-embed.png` });
    await open("Author mark refs");
    const refs = page.locator(".linked-references").first();
    await refs.locator(".ls-block.authored").first().waitFor();
    await measure("linked-references", refs, { needAuthored: 1 });
    // Zoomed into an authored parent.
    await open("Author mark gap");
    await main.locator(".ls-block.authored").filter({ hasText: "An agent child" }).first()
      .locator(":scope > .block-main .bullet-container").click();
    await page.locator(".zoomed-page, .zoomed-block").first().waitFor();
    await measure("zoom", page.locator(".page-blocks, .zoomed-block").first(), { needAuthored: 1 });
    if (width === 1280) {
      // A block that is authored and carries a margin comment on a passage of its text.
      await open("Author tint comment");
      await page.locator(".comment-quote-anchor").first().waitFor();
      await page.waitForTimeout(300);
      await measure("authored-with-comment", page.locator(".page-blocks").first(), { needAuthored: 2 });
      await page.screenshot({ path: `${out}/${tag}-comment.png` });
      if (theme === "light") {
        await open("Author mark gap");
        await page.evaluate(async name => (await import("/src/ui.ts")).openPageInSidebar(name), "Author mark gap");
        const side = page.locator(".right-sidebar");
        await side.locator(".ls-block.authored").first().waitFor();
        await measure("sidebar", side, { needAuthored: 6 });
        await page.screenshot({ path: `${out}/${tag}-sidebar.png` });
        await page.evaluate(async () => (await import("/src/ui.ts")).closeRightSidebarSafely());
        const pane = await page.evaluate(async () => (await import("/src/panes.ts")).splitPane(undefined, "row"));
        if (pane) {
          const panes = page.locator(".page-blocks");
          await panes.nth(1).locator(".ls-block.authored").first().waitFor();
          await measure("split-pane", panes.nth(1), { needAuthored: 6 });
          await page.screenshot({ path: `${out}/${tag}-split.png` });
          await page.evaluate(async () => { const p = await import("/src/panes.ts"); p.closePane(p.focusedPaneId()); });
        } else failures.push(`${tag} split-pane: splitPane returned null`);
      }
    }
    // Both editors: entering an authored block keeps its tint and does not move its first letter.
    for (const engine of ["textarea", "cm6"]) {
      await open("Author mark gap");
      await page.evaluate(async on => (await import("/src/editor/cm6Setting.ts")).setCm6EditorEnabled(on), engine === "cm6");
      if (engine === "cm6") await page.evaluate(async () => { await import("/src/components/Cm6BlockEditor.tsx"); });
      await page.waitForTimeout(300);
      const row = main.locator(".ls-block.authored").filter({ hasText: "An agent wrote this block" }).first();
      const beforeX = await row.locator(":scope > .block-main").evaluate(m => {
        const c = m.querySelector(".block-content"); const w = document.createTreeWalker(c, NodeFilter.SHOW_TEXT);
        let n; while ((n = w.nextNode())) if (n.textContent.trim() && !n.parentElement.closest(".author-chip")) break;
        const r = document.createRange(); r.setStart(n, 0); r.setEnd(n, 1); return r.getBoundingClientRect().left;
      });
      await row.locator(":scope > .block-main .block-content").first().click({ position: { x: 40, y: 8 } });
      await page.locator(engine === "cm6" ? ".ls-block.authored .cm-content" : ".ls-block.authored textarea.block-editor").first().waitFor({ timeout: 60_000 });
      await page.waitForTimeout(150);
      const rows = await main.evaluate(measureRows);
      check(`${tag} editing-${engine}`, rows.filter(x => x.editing || !x.authored), { needAuthored: 1, needEditing: true });
      const editing = rows.find(x => x.editing);
      try { assert.ok(editing && Math.abs(editing.textX - beforeX) <= 1, `${tag} editing-${engine}: first letter moved ${beforeX} -> ${editing?.textX}`); }
      catch (error) { failures.push(String(error.message)); }
      if (width === 1280) await page.screenshot({ path: `${out}/${tag}-edit-${engine}.png`, clip: { x: 240, y: 150, width: 800, height: 260 } });
      if (engine === "textarea") {
        // Esc selects the block; Shift+Down extends over its run: one selection colour, no tint under it.
        await page.keyboard.press("Escape"); await page.waitForTimeout(200);
        await page.keyboard.press("Shift+ArrowDown"); await page.waitForTimeout(200);
        const sel = await main.evaluate(measureRows);
        check(`${tag} selection`, sel, { needAuthored: 2 });
        try { assert.ok(sel.some(x => x.authored && x.selected), `${tag} selection: no authored row is selected`); }
        catch (error) { failures.push(String(error.message)); }
        if (width === 1280) await page.screenshot({ path: `${out}/${tag}-selection.png`, clip: { x: 240, y: 150, width: 800, height: 260 } });
      }
      await page.keyboard.press("Escape");
    }
    await ctx.close();
  }
  // The Show/Hide author marks switch: no tint, text where it was.
  {
    const { ctx, page, open } = await newPage(1280, "light", 2, { hidden: true });
    activePage = page;
    await open("Author mark gap");
    const main = page.locator(".page-blocks").first();
    await main.locator(".ls-block.authored .bullet-closed").first().waitFor();
    check("1280-light switch-off", await main.evaluate(measureRows), { needAuthored: 6, switchOff: true });
    await page.screenshot({ path: `${out}/1280-light-switch-off.png` });
    await ctx.close();
  }
  // 6x zoom of the bullet area.
  {
    const { ctx, page, open } = await newPage(1280, "light", 6);
    activePage = page;
    await open("Author mark gap");
    const folded = page.locator(".page-blocks .ls-block.authored:has(> .block-main .bullet-closed)").first();
    await folded.waitFor();
    const fb = await folded.locator(":scope > .block-main .bullet-container").boundingBox();
    const top = await page.locator(".page-blocks .ls-block").first().locator(":scope > .block-main .bullet-container").boundingBox();
    await page.screenshot({ path: `${out}/zoom-gap.png`, clip: { x: fb.x - 8, y: top.y - 6, width: 120, height: fb.y + fb.height - top.y + 12 } });
    await ctx.close();
  }
  console.log(JSON.stringify({ failures }, null, 1));
  assert.deepEqual(browserErrors, []);
  assert.deepEqual(failures, []);
  console.log(`GH #893 tint: ${results.length} surfaces measured; screenshots in ${out}`);
} catch (error) {
  await activePage?.screenshot({ path: `${out}/failure.png` }).catch(() => {});
  writeFileSync(`${out}/failure.json`, JSON.stringify({ commit, observed: String(error), failures, browserErrors }, null, 2));
  throw error;
} finally {
  writeFileSync(`${out}/proof.json`, JSON.stringify({ commit, journey: "author tint across surfaces",
    expected: "authored rows tinted, own rows not; no gutter line; first letter at the plain offset; run members touch; selected rows lose the tint; switch off removes it",
    results, failures, browserErrors }, null, 2));
  await browser?.close();
  await server.close();
}

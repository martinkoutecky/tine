// GH #892: the authorship thread look and the author icon, in the real browser app on the in-memory `?regressions`
// mock graph (page "Author thread"). Measures and screenshots: (a) today's tint, (b) the thread with one chip per
// run, (c) the thread with an icon, light and dark; then the Guide's own custom.css examples, taken from
// crates/tine-guide/src/templates/customize-look.md as written, must draw the same thread and the same icon.
// Run: node scripts/shot-author-thread.mjs [artifactDir]   (default artifacts/authorship-thread-892)
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createServer } from "vite";
import solid from "vite-plugin-solid";
import { chromium } from "playwright";

const out = process.argv[2] ?? "artifacts/authorship-thread-892";
const commit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
mkdirSync(out, { recursive: true });

// The Guide's two examples, as a reader pastes them.
const guide = readFileSync("crates/tine-guide/src/templates/customize-look.md", "utf8");
const section = guide.slice(guide.indexOf("## 5. Show who wrote a block"), guide.indexOf("## 6. Find out what styles"));
const blocks = [...section.matchAll(/```css\n([\s\S]*?)```/g)].map(m => m[1].replace(/^ {4}/gm, ""));
assert.equal(blocks.length, 2, "the Guide section has a thread example and an icon example");
const [guideThread, guideIcon] = blocks;
// The same icon in the Tine-drawn run, set the documented way (on the chip).
const iconCss = guideIcon;

const server = await createServer({ configFile: false, plugins: [solid()],
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("author-thread"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, watch: { ignored: ["**/target/**", "**/.toolchain/**", "**/.cargo/**", "**/artifacts/**"] } },
});
await server.listen();
let browser, activePage;
const failures = [], browserErrors = [], results = {};

/** Rows of the page with their geometry; the thread line is the wrapper's ::after. */
function measure() {
  const rows = [];
  for (const row of document.querySelectorAll(".page-blocks .ls-block")) {
    const wrapper = row.querySelector(":scope > .block-main > .block-content-wrapper");
    if (!wrapper) continue;
    const chip = wrapper.querySelector(":scope > .block-content > .author-chip");
    // The thread's line is the text box's ::after (never the row's own, which the thread-lines decoration draws on),
    // placed against the full row.
    const main = row.querySelector(":scope > .block-main");
    const after = getComputedStyle(wrapper, "::after");
    const w = wrapper.getBoundingClientRect();
    const text = (() => {
      const walker = document.createTreeWalker(wrapper.querySelector(".block-content") ?? wrapper, NodeFilter.SHOW_TEXT);
      let n; while ((n = walker.nextNode())) if (n.textContent.trim() && !n.parentElement.closest(".author-chip, .block-refs-count")) {
        const at = n.textContent.search(/\S/), r = document.createRange(); r.setStart(n, at); r.setEnd(n, at + 1);
        return { x: r.getBoundingClientRect().left, label: n.textContent.trim().slice(0, 30) };
      }
      return { x: null, label: "" };
    })();
    const c = chip && chip.offsetParent ? chip.getBoundingClientRect() : null;
    const hasLine = after.content !== "none" && after.content !== "normal" && after.width === "2px" && after.position === "absolute";
    rows.push({ label: text.label, textX: text.x, authored: row.classList.contains("authored"), tinted: row.classList.contains("author-tinted"),
      up: row.classList.contains("author-run-continued"), down: row.classList.contains("author-run-continues"),
      bg: getComputedStyle(wrapper).backgroundColor, line: hasLine, lineTop: after.top, lineBottom: after.bottom,
      lineColor: after.backgroundColor, lineRight: hasLine ? parseFloat(after.right) : null,
      rowRight: main.getBoundingClientRect().right, wrapperTop: w.top, wrapperBottom: w.bottom,
      chip: c && { left: c.left, right: c.right, top: c.top, bottom: c.bottom, width: c.width, height: c.height,
        image: getComputedStyle(chip).backgroundImage, author: chip.getAttribute("data-author"), text: chip.textContent } });
  }
  return rows;
}
/** GH #892 review F2: where the thread's line and chip sit measured from the FULL row's right edge (`.block-main`),
 *  and the trailing comment controls (marker, resolved label, folded "N below" chip) of each authored row. The
 *  line's box is the pseudo-element's offset from its containing block (the wrapper before the fix, the row after). */
function trailing() {
  const rows = [];
  for (const row of document.querySelectorAll(".page-blocks .ls-block.author-tinted")) {
    const main = row.querySelector(":scope > .block-main"), wrapper = main.querySelector(":scope > .block-content-wrapper");
    // The line is the text box's ::after; its containing block is the nearest positioned ancestor (the full row once
    // the text box is not itself positioned, the text box before the F2 fix).
    const after = getComputedStyle(wrapper, "::after");
    const cb = getComputedStyle(wrapper).position !== "static" ? wrapper : wrapper.offsetParent;
    const rowRight = main.getBoundingClientRect().right;
    const lineRight = cb.getBoundingClientRect().right - parseFloat(after.right);
    const chip = wrapper.querySelector(".author-chip");
    const controls = [...main.querySelectorAll(":scope > .comment-entry > button, :scope > .comment-below-chip")]
      .filter(el => el.offsetParent).map(el => ({ kind: el.className.split(" ")[0], left: el.getBoundingClientRect().left, right: el.getBoundingClientRect().right }));
    const c = chip && chip.offsetParent ? chip.getBoundingClientRect() : null;
    rows.push({ label: wrapper.querySelector(".block-content")?.textContent.trim().slice(0, 28) ?? "", lineGap: rowRight - lineRight, lineLeft: lineRight - 2,
      chipGap: c ? rowRight - c.right : null, chipLeft: c?.left ?? null, controls });
  }
  return rows;
}
const transparent = bg => bg === "rgba(0, 0, 0, 0)" || bg === "transparent";
const near = (a, b, tol = 1) => Math.abs(a - b) <= tol;
const run = async (name, fn) => { try { await fn(); } catch (error) { failures.push(`${name}: ${error.message ?? error}`); } };

async function newPage(width, theme, dsf = 2, pageName = "Author thread", threadLines = false) {
  const ctx = await browser.newContext({ viewport: { width, height: 900 }, hasTouch: width === 390, isMobile: width === 390, deviceScaleFactor: dsf });
  const page = await ctx.newPage();
  page.setDefaultTimeout(15_000);
  page.on("pageerror", error => browserErrors.push(String(error)));
  await page.addInitScript(() => localStorage.setItem("logseq-claude.sidebarOpen", "0"));
  await page.goto(`${server.resolvedUrls.local[0]}?regressions`);
  await page.locator(".ls-block").first().waitFor();
  await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
  // The thread-lines decoration is a plugin's declarative decoration; switch it on the way its own fixture does.
  if (threadLines) await page.evaluate(async () => {
    const { pluginManager } = await import("/src/plugins/manager.ts");
    pluginManager.hasDeclarativeDecoration = kind => kind === "thread-lines";
    pluginManager.declarativeDecorationSetting = () => undefined;
  });
  await page.addStyleTag({ content: ".toast-stack { display: none !important; }" });
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill(pageName);
  await page.locator(".switcher-row").filter({ hasText: pageName }).first().click();
  await page.locator(".page-title").filter({ hasText: pageName }).first().waitFor();
  if (await page.locator(".mobile-drawer-scrim").count()) await page.keyboard.press("Escape");
  await page.locator(".ls-block.authored").first().waitFor();
  await page.evaluate(() => document.fonts.ready);
  await page.mouse.move(0, 0);
  return { ctx, page };
}
const shoot = (page, file) => page.locator(".page-blocks").first().screenshot({ path: `${out}/${file}` });
/** The thread look the way a user gets it: Settings > Appearance > Advanced > Fine-tuning > Author marks. The class
 *  `tine-author-marks-thread` on <html> is what the knob produces; assert it, never set it by hand. */
async function setThreadClass(page, on) {
  await page.locator('button.icon-btn[title^="Settings"], .mobile-drawer-settings, [aria-label^="Settings"]').first().click();
  await page.waitForSelector(".settings-modal", { timeout: 5000 });
  const appearance = page.locator(".settings-modal").getByText("Appearance", { exact: true }).first();
  if (await appearance.count()) await appearance.click();
  const advanced = page.locator(".settings-advanced-toggle");
  if (await advanced.count()) await advanced.first().click();
  const knob = page.locator('select[aria-label="Author marks"]');
  await knob.waitFor({ timeout: 5000 });
  await knob.evaluate(el => el.scrollIntoView({ block: "center" }));
  await knob.selectOption(on ? "thread" : "tint");
  await page.locator(".settings-modal").press("Escape");
  await page.locator(".settings-modal").waitFor({ state: "detached", timeout: 3000 }).catch(async () => {
    await page.keyboard.press("Escape");
    await page.locator(".settings-modal").waitFor({ state: "detached", timeout: 5000 });
  });
  const has = await page.evaluate(() => document.documentElement.classList.contains("tine-author-marks-thread"));
  assert.equal(has, on, `Author marks = ${on ? "Thread" : "Tint"} must ${on ? "add" : "remove"} html.tine-author-marks-thread`);
  await page.mouse.move(0, 0);
}
const addCss = (page, css) => page.addStyleTag({ content: css });

try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const theme of ["light", "dark"]) {
    const { ctx, page } = await newPage(1280, theme);
    activePage = page;
    // (a) today's tint.
    const tint = await page.evaluate(measure);
    await shoot(page, `${theme}-a-tint.png`);
    await run(`${theme} tint`, () => {
      const authored = tint.filter(r => r.authored);
      assert.ok(authored.length >= 8, `expected 8 authored rows, saw ${authored.length}`);
      for (const r of authored) { assert.ok(r.tinted && !transparent(r.bg), `"${r.label}" has no tint`); assert.ok(!r.line, `"${r.label}" draws a thread line in the tint look`); }
    });
    // (b) the thread: one class on <html>.
    await setThreadClass(page, true);
    const thread = await page.evaluate(measure);
    await shoot(page, `${theme}-b-thread.png`);
    results[`${theme}-thread`] = thread;
    await run(`${theme} thread`, () => {
      const authored = thread.filter(r => r.authored);
      for (const r of authored) {
        assert.ok(transparent(r.bg), `"${r.label}": the tint is still drawn under the thread`);
        assert.ok(r.line, `"${r.label}": no thread line`);
      }
      // Text column untouched: every first letter is where the tint look had it, authored or not.
      for (let i = 0; i < tint.length; i++) assert.ok(near(tint[i].textX, thread[i].textX, 0.5), `"${tint[i].label}": text moved ${tint[i].textX} -> ${thread[i].textX}`);
      // One chip per run: Claude's and Mira's, in the gutter right of the text column, at one fixed distance from the right edge.
      const chips = authored.filter(r => r.chip);
      assert.deepEqual(chips.map(r => r.chip.author), ["Claude", "Mira"], "one chip per run");
      assert.ok(near(chips[0].chip.right, chips[1].chip.right, 0.5), "chips are not at one distance from the right edge");
      for (const r of chips) assert.ok(r.chip.left > r.textX, `"${r.label}": chip is left of its text`);
      // The line is one vertical through the chips: same x on every row of a run, under the chip's centre, joined across rows.
      for (const r of authored) {
        assert.ok(near(r.rowRight, authored[0].rowRight, 0.5), `"${r.label}": row edge differs`);
        // The line is placed against the full row: a continued edge is the row's own edge, which meets the next row's.
        assert.equal(r.up ? r.lineTop : "", r.up ? "0px" : "", `"${r.label}": run-continued line does not reach the row above`);
        assert.equal(r.down ? r.lineBottom : "", r.down ? "0px" : "", `"${r.label}": run-continues line does not reach the row below`);
      }
      for (const r of chips) assert.ok(r.chip.left < r.rowRight - 7 && r.chip.right > r.rowRight - 8, `"${r.label}": the line is not under its chip`);
      // The runs cross depth: Claude's five rows are one run (parent, child, grandchild, child, closing line), Mira's four likewise.
      for (const r of authored) assert.ok(near(r.lineRight, authored[0].lineRight, 0.5), `"${r.label}": the line is not at one distance from the right edge`);
      const claude = authored.slice(0, 5), ada = authored.slice(5);
      assert.deepEqual(claude.map(r => [r.up, r.down]), [[false, true], [true, true], [true, true], [true, true], [true, false]], "Claude's run");
      assert.deepEqual(ada.map(r => [r.up, r.down]), [[false, true], [true, true], [true, true], [true, false]], "Mira's run");
    });
    // (c) the thread with an icon for Claude: both tokens on the chip, as the Guide says.
    await addCss(page, iconCss);
    const icon = await page.evaluate(measure);
    await shoot(page, `${theme}-c-thread-icon.png`);
    await run(`${theme} icon`, () => {
      const claude = icon.find(r => r.chip?.author === "Claude"), ada = icon.find(r => r.chip?.author === "Mira");
      assert.ok(claude.chip.image.includes("data:image/svg+xml"), "Claude's chip has no icon image");
      assert.ok(near(claude.chip.width, 16, 0.5) && near(claude.chip.height, 16, 0.5), `icon chip is ${claude.chip.width}x${claude.chip.height}`);
      assert.equal(ada.chip.image, "none", "Mira's chip got Claude's icon");
      assert.ok(ada.chip.width > 20, "Mira's chip lost its name");
      assert.ok(near(claude.chip.right, ada.chip.right, 0.5), "the icon chip left the gutter's edge");
    });
    // Without the thread class: the icon is also drawn in the tint look.
    await setThreadClass(page, false);
    const tintIcon = await page.evaluate(measure);
    await run(`${theme} tint icon`, () => assert.ok(tintIcon.find(r => r.chip?.author === "Claude").chip.image.includes("data:image/svg+xml")));
    // The Guide's thread example, pasted as written into a tint-look page, draws the same thread.
    await ctx.close();
    const { ctx: ctx2, page: page2 } = await newPage(1280, theme);
    activePage = page2;
    await addCss(page2, guideThread);
    const guided = await page2.evaluate(measure);
    await shoot(page2, `${theme}-b-thread-guide-css.png`);
    await run(`${theme} guide thread`, () => {
      const authored = guided.filter(r => r.authored);
      for (const r of authored) { assert.ok(transparent(r.bg), `"${r.label}": the Guide's example leaves the tint`); assert.ok(r.line, `"${r.label}": the Guide's example draws no line`); }
      for (let i = 0; i < tint.length; i++) assert.ok(near(tint[i].textX, guided[i].textX, 0.5), `Guide example moves "${tint[i].label}"`);
      assert.deepEqual(authored.filter(r => r.chip).map(r => r.chip.author), ["Claude", "Mira"], "Guide example: one chip per run");
      for (let i = 0; i < thread.length; i++) if (thread[i].authored) {
        assert.equal(guided[i].up, thread[i].up); assert.equal(guided[i].down, thread[i].down);
        assert.ok(near(guided[i].rowRight, thread[i].rowRight, 0.5));
      }
    });
    await addCss(page2, guideIcon);
    const guidedIcon = await page2.evaluate(measure);
    await run(`${theme} guide icon`, () => assert.ok(guidedIcon.find(r => r.chip?.author === "Claude").chip.image.includes("data:image/svg+xml")));
    await ctx2.close();
  }
  // The phone: the thread keeps clear of the 44px fold area (screenshots, and the line's distance).
  for (const theme of ["light", "dark"]) {
    const { ctx, page } = await newPage(390, theme);
    activePage = page;
    await setThreadClass(page, true);
    const rows = await page.evaluate(measure);
    await shoot(page, `phone-${theme}-b-thread.png`);
    await run(`phone ${theme}`, () => {
      for (const r of rows.filter(r => r.authored && r.chip)) assert.ok(r.rowRight - r.chip.right >= 40, `"${r.label}": chip is ${r.rowRight - r.chip.right}px from the edge, under the fold area`);
    });
    await ctx.close();
  }
  // GH #892 review F2: trailing comment controls never move the thread. One run of rows, with and without a marker,
  // a resolved label and a folded "N below" chip: the line and the chip keep one distance from the row's right edge,
  // and no control sits under either. Phone (tap), tablet and desktop widths.
  for (const width of [390, 800, 1280]) {
    const { ctx, page } = await newPage(width, "light", 2, "Author thread trailing");
    activePage = page;
    await setThreadClass(page, true);
    await page.locator(".comment-below-chip").first().waitFor({ timeout: 5000 }).catch(() => {});
    // The page's "N resolved" control reveals the closed thread; narrower than the margin column it becomes a label.
    if (await page.locator(".comment-resolved-toggle").count()) await page.locator(".comment-resolved-toggle").first().click();
    await page.mouse.move(0, 0);
    const rows = await page.evaluate(trailing);
    await shoot(page, `trailing-${width}.png`);
    results[`trailing-${width}`] = rows;
    await run(`trailing controls at ${width}px`, () => {
      assert.equal(rows.length, 5, "the five authored rows of the page");
      const kinds = new Set(rows.flatMap(r => r.controls.map(c => c.kind)));
      assert.ok(kinds.has("comment-below-chip"), `the folded row's "N below" chip is drawn at ${width}px (saw ${[...kinds]})`);
      if (width < 1000) for (const kind of ["margin-marker", "comment-resolved-entry"]) assert.ok(kinds.has(kind), `${kind} is drawn at ${width}px (saw ${[...kinds]})`);
      for (const r of rows) {
        assert.ok(near(r.lineGap, rows[0].lineGap, 0.5), `"${r.label}": the line is ${r.lineGap}px from the row's edge, the plain row's is ${rows[0].lineGap}px (controls: ${r.controls.map(c => c.kind)})`);
        if (r.chipGap !== null) assert.ok(near(r.chipGap, rows[0].chipGap, 0.5), `"${r.label}": the chip is ${r.chipGap}px from the row's edge, the plain row's is ${rows[0].chipGap}px`);
        for (const c of r.controls) assert.ok(c.right <= r.lineLeft + 0.5 && (r.chipLeft === null || c.right <= r.chipLeft + 0.5), `"${r.label}": ${c.kind} (${c.left}..${c.right}) reaches the thread at ${r.lineLeft}`);
      }
    });
    await ctx.close();
  }
  // The thread look shares no pseudo-element with the thread-lines decoration, which draws a parent's vertical line on
  // the row's own ::after and each child's elbow on its ::before: with both on, an authored parent that has children
  // draws both lines (the decoration's and the author thread's).
  for (const theme of ["light", "dark"]) {
    const { ctx, page } = await newPage(1280, theme, 2, "Author thread", true);
    activePage = page;
    await setThreadClass(page, true);
    const facts = await page.evaluate(() => {
      const rows = [...document.querySelectorAll(".page-blocks .ls-block.author-tinted.plugin-thread-lines:not(.collapsed)")]
        .filter(row => row.querySelector(":scope > .block-children-container .ls-block"));
      return rows.map(row => {
        const main = row.querySelector(":scope > .block-main"), wrapper = main.querySelector(":scope > .block-content-wrapper");
        const guide = getComputedStyle(main, "::after"), author = getComputedStyle(wrapper, "::after");
        const child = row.querySelector(":scope > .block-children-container .ls-block > .block-main");
        const elbow = getComputedStyle(child, "::before");
        return { label: wrapper.textContent.trim().slice(0, 28),
          guide: { content: guide.content, position: guide.position, display: guide.display, width: parseFloat(guide.borderInlineStartWidth) },
          author: { content: author.content, position: author.position, width: author.width, display: author.display },
          elbow: { content: elbow.content, display: elbow.display, width: parseFloat(elbow.borderInlineStartWidth) } };
      });
    });
    await shoot(page, `${theme}-thread-with-thread-lines.png`);
    results[`${theme}-thread-lines`] = facts;
    await run(`${theme} thread look with thread-lines on`, () => {
      assert.ok(facts.length >= 2, `expected authored parents drawn with the thread-lines decoration, saw ${facts.length}`);
      for (const f of facts) {
        assert.ok(f.guide.content !== "none" && f.guide.content !== "normal" && f.guide.position === "absolute" && f.guide.display !== "none" && f.guide.width > 0, `"${f.label}": the decoration's parent line is not drawn (${JSON.stringify(f.guide)})`);
        assert.ok(f.elbow.content !== "none" && f.elbow.display !== "none" && f.elbow.width > 0, `"${f.label}": the child's elbow is not drawn (${JSON.stringify(f.elbow)})`);
        assert.ok(f.author.content !== "none" && f.author.content !== "normal" && f.author.position === "absolute" && f.author.width === "2px" && f.author.display !== "none", `"${f.label}": the author thread line is not drawn (${JSON.stringify(f.author)})`);
      }
    });
    await ctx.close();
  }
  console.log(JSON.stringify({ failures }, null, 1));
  assert.deepEqual(browserErrors, []);
  assert.deepEqual(failures, []);
  console.log(`GH #892 thread look: screenshots in ${out}`);
} catch (error) {
  await activePage?.screenshot({ path: `${out}/failure.png` }).catch(() => {});
  writeFileSync(`${out}/failure.json`, JSON.stringify({ commit, observed: String(error), failures, browserErrors }, null, 2));
  throw error;
} finally {
  writeFileSync(`${out}/proof.json`, JSON.stringify({ commit, journey: "authorship thread look and icon",
    expected: "tint look unchanged; thread class replaces it with a 2px line through one chip per run, text column unmoved; icon chip is 16x16; the Guide's examples draw the same",
    failures, browserErrors, results }, null, 2));
  await browser?.close();
  await server.close();
}

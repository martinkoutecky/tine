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
      wrapperRight: w.right, wrapperTop: w.top, wrapperBottom: w.bottom,
      chip: c && { left: c.left, right: c.right, top: c.top, bottom: c.bottom, width: c.width, height: c.height,
        image: getComputedStyle(chip).backgroundImage, author: chip.getAttribute("data-author"), text: chip.textContent } });
  }
  return rows;
}
const transparent = bg => bg === "rgba(0, 0, 0, 0)" || bg === "transparent";
const near = (a, b, tol = 1) => Math.abs(a - b) <= tol;
const run = async (name, fn) => { try { await fn(); } catch (error) { failures.push(`${name}: ${error.message ?? error}`); } };

async function newPage(width, theme, dsf = 2) {
  const ctx = await browser.newContext({ viewport: { width, height: 900 }, hasTouch: width === 390, isMobile: width === 390, deviceScaleFactor: dsf });
  const page = await ctx.newPage();
  page.setDefaultTimeout(15_000);
  page.on("pageerror", error => browserErrors.push(String(error)));
  await page.addInitScript(() => localStorage.setItem("logseq-claude.sidebarOpen", "0"));
  await page.goto(`${server.resolvedUrls.local[0]}?regressions`);
  await page.locator(".ls-block").first().waitFor();
  await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
  await page.addStyleTag({ content: ".toast-stack { display: none !important; }" });
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("Author thread");
  await page.locator(".switcher-row").filter({ hasText: "Author thread" }).first().click();
  await page.locator(".page-title").filter({ hasText: "Author thread" }).first().waitFor();
  if (await page.locator(".mobile-drawer-scrim").count()) await page.keyboard.press("Escape");
  await page.locator(".ls-block.authored").first().waitFor();
  await page.evaluate(() => document.fonts.ready);
  await page.mouse.move(0, 0);
  return { ctx, page };
}
const shoot = (page, file) => page.locator(".page-blocks").first().screenshot({ path: `${out}/${file}` });
const setThreadClass = (page, on) => page.evaluate(on => document.documentElement.classList.toggle("tine-author-thread", on), on);
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
        assert.ok(near(r.wrapperRight, authored[0].wrapperRight, 0.5), `"${r.label}": row edge differs`);
        assert.equal(r.up ? r.lineTop : "", r.up ? "-1px" : "", `"${r.label}": run-continued line does not reach the row above`);
        assert.equal(r.down ? r.lineBottom : "", r.down ? "-1px" : "", `"${r.label}": run-continues line does not reach the row below`);
      }
      for (const r of chips) assert.ok(r.chip.left < r.wrapperRight - 7 && r.chip.right > r.wrapperRight - 8, `"${r.label}": the line is not under its chip`);
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
        assert.ok(near(guided[i].wrapperRight, thread[i].wrapperRight, 0.5));
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
      for (const r of rows.filter(r => r.authored && r.chip)) assert.ok(r.wrapperRight - r.chip.right >= 40, `"${r.label}": chip is ${r.wrapperRight - r.chip.right}px from the edge, under the fold area`);
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

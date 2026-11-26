// GH #994, #1024, #1025: three looks in the real browser app on the in-memory `?regressions` mock graph, at a phone
// width (390px, touch: no hover, coarse pointer) and a desktop width (1280px, mouse).
//  #994  the thread look's strip: row end padding, the author chip, and the Linked References buttons' right edge
//        with Author marks = Thread against Tint.
//  #1024 the toolbar's Workspace control (sidebar closed, so the toolbar mount shows).
//  #1025 the page title's actions: which are visible without hover, their order, and where they wrap (a long title).
// Writes PNGs and measurements.json to the output dir; asserts nothing, so it can run on an older checkout too.
// Run: node scripts/shot-gh994-1024-1025-looks.mjs [outDir]   (default artifacts/gh994-1024-1025-looks)
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { createServer } from "vite";
import solid from "vite-plugin-solid";
import { chromium } from "playwright";

const out = process.argv[2] ?? "artifacts/gh994-1024-1025-looks";
mkdirSync(out, { recursive: true });
const server = await createServer({ configFile: false, plugins: [solid()],
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("looks"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, fs: { allow: [process.cwd(), realpathSync("node_modules")] }, watch: { ignored: ["**/target/**", "**/.toolchain/**", "**/.cargo/**", "**/artifacts/**"] } },
});
await server.listen();
const results = {};
let browser;

async function open(width, pageName) {
  const phone = width <= 640;
  const ctx = await browser.newContext({ viewport: { width, height: 900 }, hasTouch: phone, isMobile: phone, deviceScaleFactor: 2 });
  const page = await ctx.newPage();
  page.setDefaultTimeout(15_000);
  await page.addInitScript(() => localStorage.setItem("logseq-claude.sidebarOpen", "0"));
  await page.goto(`${server.resolvedUrls.local[0]}?regressions`);
  await page.locator(".ls-block").first().waitFor();
  await page.addStyleTag({ content: ".toast-stack { display: none !important; }" });
  if (pageName) await go(page, pageName);
  await page.evaluate(() => document.fonts.ready);
  await page.mouse.move(0, 0);
  return { ctx, page };
}
async function go(page, pageName) {
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill(pageName);
  await page.locator(".switcher-row").filter({ hasText: pageName }).first().click();
  await page.locator(".page-title").filter({ hasText: pageName }).first().waitFor();
  if (await page.locator(".mobile-drawer-scrim").count()) await page.keyboard.press("Escape");
}
async function setThread(page, on) {
  await page.locator('button.icon-btn[title^="Settings"], .mobile-drawer-settings, [aria-label^="Settings"]').first().click();
  await page.waitForSelector(".settings-modal", { timeout: 5000 });
  const appearance = page.locator(".settings-modal").getByText("Appearance", { exact: true }).first();
  if (await appearance.count()) await appearance.click();
  const advanced = page.locator(".settings-advanced-toggle");
  if (await advanced.count()) await advanced.first().click();
  const knob = page.locator('select[aria-label="Author marks"]');
  await knob.evaluate(el => el.scrollIntoView({ block: "center" }));
  await knob.selectOption(on ? "thread" : "tint");
  await page.keyboard.press("Escape");
  await page.locator(".settings-modal").waitFor({ state: "detached", timeout: 5000 }).catch(() => page.keyboard.press("Escape"));
  await page.mouse.move(0, 0);
}
// The actions fade through --reveal in 0.1s: measure once every action has settled.
const settled = page => page.waitForFunction(() => [...document.querySelectorAll(".page-title-actions > *")]
  .every(el => ["0", "1"].includes(getComputedStyle(el).opacity)));
const titleActions = async page => (await settled(page), page.evaluate(() => {
  const row = document.querySelector(".page-title-row");
  const title = row.querySelector(".page-title").getBoundingClientRect();
  const actions = [...row.querySelectorAll(".page-title-actions > *")].map(el => {
    const r = el.getBoundingClientRect();
    return { cls: el.className.split(" ")[0], opacity: Number(getComputedStyle(el).opacity), left: Math.round(r.left), top: Math.round(r.top), right: Math.round(r.right) };
  });
  return { titleLeft: Math.round(title.left), titleBottom: Math.round(title.bottom), actions };
}));
const shootTitle = async (page, file) => {
  const box = await page.locator(".page-title-row").first().boundingBox();
  await page.screenshot({ path: `${out}/${file}`, clip: { x: 0, y: Math.max(0, box.y - 8), width: page.viewportSize().width, height: box.height + 16 } });
};

try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const width of [390, 1280]) {
    const tag = width <= 640 ? "phone" : "desktop";
    // #1024: the toolbar, with the Workspace control chosen onto the bar as the reporter has it.
    {
      const { ctx, page } = await open(width);
      await page.evaluate(async () => {
        const { saveToolbarChoice } = await import("/src/toolbarChoice.ts");
        saveToolbarChoice({ bar: ["workspaces", "right-sidebar", "settings"], menu: ["journals", "calendar", "share", "theme"] });
      });
      const control = page.locator('header.topbar [data-workspace-switcher-compact="true"]').first();
      await control.waitFor();
      results[`1024-${tag}`] = await control.evaluate(el => ({
        width: Math.round(el.getBoundingClientRect().width),
        caret: el.querySelectorAll(".workspace-switcher-caret").length,
        buttonClass: el.querySelector("button").className,
        markBackground: getComputedStyle(el.querySelector(".workspace-switcher-mark")).backgroundColor,
      }));
      await page.locator("header.topbar").screenshot({ path: `${out}/1024-${tag}-toolbar.png` });
      if (width > 640) {
        // The sidebar's labelled switcher, which this change leaves as it was.
        await page.evaluate(async () => (await import("/src/ui.ts")).setLeftSidebarOpen(true));
        const sidebar = page.locator("[data-workspace-switcher-sidebar] [data-workspace-switcher]").first();
        await sidebar.waitFor();
        results[`1024-${tag}-sidebar`] = await sidebar.evaluate(el => ({ caret: el.querySelectorAll(".workspace-switcher-caret").length,
          buttonClass: el.querySelector("button").className, markBackground: getComputedStyle(el.querySelector(".workspace-switcher-mark")).backgroundColor }));
        await sidebar.screenshot({ path: `${out}/1024-${tag}-sidebar.png` });
      }
      await ctx.close();
    }
    // #1025: the title actions on a tag page (Tables chip, menu, Share, star), short and long title.
    {
      const { ctx, page } = await open(width, "project");
      // The browser mock has no tag index, so the Tables chip shows only with the table switched on (an active chip
      // stays in view on desktop too): switch it on the way the chip does, through the page property.
      await page.evaluate(async () => (await import("/src/document/index.ts")).setPageProperty("project", "tine.tag-table", "true"));
      await page.locator(".tag-table-toggle").first().waitFor({ state: "attached" });
      await page.mouse.move(0, 0);
      results[`1025-${tag}-rest`] = await titleActions(page);
      await shootTitle(page, `1025-${tag}-short-title.png`);
      if (width > 640) {
        await page.locator(".page-title").first().hover();
        await page.waitForFunction(() => getComputedStyle(document.querySelector(".page-share-btn")).opacity === "1", null, { timeout: 2000 }).catch(() => {});
        results[`1025-${tag}-hover`] = await titleActions(page);
        await shootTitle(page, `1025-${tag}-short-title-hover.png`);
        await page.mouse.move(0, 0);
        await page.waitForFunction(() => getComputedStyle(document.querySelector(".page-share-btn")).opacity === "0", null, { timeout: 2000 }).catch(() => {});
      }
      if (width > 640) {
        // Keyboard focus in the row reveals the actions; an open menu keeps its button in view.
        await page.locator(".page-share-btn").first().focus();
        await page.waitForFunction(() => getComputedStyle(document.querySelector(".fav-star")).opacity === "1", null, { timeout: 2000 }).catch(() => {});
        results[`1025-${tag}-focus`] = await titleActions(page);
        await page.locator(".page-share-btn").first().evaluate(el => el.blur());
        await page.locator(".page-actions-trigger").first().click();
        await page.mouse.move(0, 0);
        await page.waitForFunction(() => getComputedStyle(document.querySelector(".fav-star")).opacity === "0", null, { timeout: 2000 }).catch(() => {});
        results[`1025-${tag}-menu-open`] = await titleActions(page);
        await shootTitle(page, `1025-${tag}-menu-open.png`);
        await page.keyboard.press("Escape");
      }
      // A long title, as a long page name would render (the title text only; layout is what is checked).
      await page.locator(".page-title").first().evaluate(h => { h.lastChild.textContent = "Project notes and the plans that go with them"; });
      results[`1025-${tag}-long`] = await titleActions(page);
      await shootTitle(page, `1025-${tag}-long-title.png`);
      await ctx.close();
    }
    // #994: the thread strip, and the Linked References buttons with Thread against Tint.
    {
      const { ctx, page } = await open(width, "Author thread");
      await setThread(page, true);
      await page.locator(".ls-block.authored").first().waitFor();
      results[`994-${tag}-thread`] = await page.evaluate(() => {
        const row = document.querySelector(".page-blocks .ls-block > .block-main");
        const chip = document.querySelector(".page-blocks .author-chip");
        return { rowEndPadding: parseFloat(getComputedStyle(row).paddingInlineEnd), chipWidth: chip ? Math.round(chip.getBoundingClientRect().width) : null };
      });
      await page.locator(".page-blocks").first().screenshot({ path: `${out}/994-${tag}-thread.png` });
      await go(page, "writing");
      const refsRight = () => page.evaluate(() => {
        const buttons = [...document.querySelectorAll(".linked-references .references-header button")];
        const blocks = document.querySelector(".linked-references .ls-block > .block-main");
        return { buttonsRight: Math.round(Math.max(...buttons.map(b => b.getBoundingClientRect().right))), firstRefRowEndPadding: blocks ? parseFloat(getComputedStyle(blocks).paddingInlineEnd) : null };
      });
      await page.locator(".linked-references").first().waitFor();
      results[`994-${tag}-refs-thread`] = await refsRight();
      await page.locator(".linked-references").first().screenshot({ path: `${out}/994-${tag}-refs-thread.png` });
      await setThread(page, false);
      results[`994-${tag}-refs-tint`] = await refsRight();
      await ctx.close();
    }
  }
} finally {
  writeFileSync(`${out}/measurements.json`, JSON.stringify(results, null, 2));
  await browser?.close();
  await server.close();
}
console.log(JSON.stringify(results, null, 1));

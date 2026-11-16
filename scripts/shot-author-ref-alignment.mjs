// GH #883: the real browser app, routed page -> reference panel, sidebar, zoom,
// block/page embeds. The mock graph is disposable and stays entirely in memory.
// Run: node scripts/shot-author-ref-alignment.mjs [artifactDir]
// AUTHOR_ALIGNMENT_BASE=<commit> serves that revision's Block.tsx for fail-before.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createServer } from "vite";
import solid from "vite-plugin-solid";
import { chromium } from "playwright";

const out = process.argv[2] ?? "target/artifacts/fix-beta-883-mb/after";
const baseline = process.env.AUTHOR_ALIGNMENT_BASE;
const commit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const source = "Author alignment regression";
const ids = [2, 3, 4].map(n => `88300000-0000-4000-8000-${String(n).padStart(12, "0")}`);
mkdirSync(out, { recursive: true });
// No PDF is opened: as in shot-interface-zoom, omit the decoder build plugin.
const server = await createServer({ configFile: false,
  plugins: [{ name: "author-alignment-baseline", enforce: "pre", load(id) {
    if (baseline && id === `${process.cwd()}/src/components/Block.tsx`)
      return execFileSync("git", ["show", `${baseline}:src/components/Block.tsx`], { encoding: "utf8" });
  } }, solid()],
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("author-alignment-proof"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, watch: { ignored: ["**/target/**", "**/.toolchain/**", "**/.cargo/**"] } },
});
await server.listen();
let browser, activePage;
const results = [], failures = [], browserErrors = [];
const proof = () => ({ commit, baseline: baseline ?? null, journey: "routed authored blocks with zero/one/twelve referrers",
  expected: "Author left edges agree within 1px; reference buttons sit beside them on the first line and still toggle referrers",
  results, failures, browserErrors });
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const [width, theme, dir] of [[1280, "light", "ltr"], [1280, "dark", "ltr"], [390, "light", "ltr"], [390, "dark", "ltr"], [1280, "light", "rtl"]]) {
    const page = activePage = await browser.newPage({ viewport: { width, height: 900 }, hasTouch: width === 390 });
    page.setDefaultTimeout(15_000);
    page.on("pageerror", error => browserErrors.push(String(error)));
    await page.addInitScript(() => localStorage.setItem("logseq-claude.sidebarOpen", "0"));
    await page.goto(`${server.resolvedUrls.local[0]}?regressions`);
    await page.locator(".ls-block").first().waitFor();
    await page.evaluate(({ theme, dir }) => {
      document.documentElement.dataset.theme = theme;
      document.documentElement.dir = dir;
    }, { theme, dir });
    const open = async name => {
      await page.keyboard.press("Control+k");
      await page.locator(".switcher-input").fill(name);
      await page.locator(".switcher-row").filter({ hasText: name }).first().click();
      await page.locator(".page-title").filter({ hasText: name }).first().waitFor();
      if (await page.locator(".mobile-drawer-scrim").count()) await page.keyboard.press("Escape");
    };
    const check = async (surface, scope) => {
      const name = `${width}-${theme}-${dir}-${surface}`;
      const row = id => scope.locator(`.ls-block[data-block-ref="${id}"] > .block-main > .block-content-wrapper > .block-content`).first();
      await row(ids[2]).locator(".block-refs-count").waitFor();
      await row(ids[0]).scrollIntoViewIfNeeded();
      await page.evaluate(() => document.fonts.ready);
      const facts = [];
      for (const id of ids) facts.push(await row(id).evaluate(el => {
        const rect = node => node?.getBoundingClientRect().toJSON() ?? null;
        return { id: el.closest(".ls-block").dataset.blockRef, content: rect(el),
          author: rect(el.querySelector(":scope > .author-chip")), badge: rect(el.querySelector(":scope > .block-refs-count")),
          count: el.querySelector(":scope > .block-refs-count")?.textContent ?? "0" };
      }));
      await page.screenshot({ path: `${out}/${name}.png` });
      const leftSpread = Math.max(...facts.map(f => f.author?.left ?? Infinity)) - Math.min(...facts.map(f => f.author?.left ?? -Infinity));
      results.push({ name, leftSpread, facts });
      try {
        assert.deepEqual(facts.map(f => f.count), ["0", "1", "12"]);
        assert.ok(leftSpread <= 1, `${name}: author left edges differ by ${leftSpread}px`);
        for (const f of facts) {
          assert.ok(f.author, `${name}: missing author`);
          if (f.badge) {
            assert.ok(Math.abs(f.badge.top - f.author.top) <= 1, `${name}: badge left the first line`);
            assert.ok(f.badge.right <= f.author.left || f.author.right <= f.badge.left, `${name}: metadata overlaps`);
          }
        }
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${name}: viewport overflow`);
      } catch (error) { failures.push(String(error)); }
      // Preserve OG's plain-click toggle, without entering the block editor.
      const badge = row(ids[1]).locator(".block-refs-count");
      await badge.click();
      const block = scope.locator(`.ls-block[data-block-ref="${ids[1]}"]`).first();
      await block.locator(":scope > .block-references").waitFor();
      assert.equal(await row(ids[1]).count(), 1, `${name}: clicking refs entered the editor`);
      await badge.click();
      await block.locator(":scope > .block-references").waitFor({ state: "detached" });
      console.log(JSON.stringify({ name, leftSpread, leftEdges: facts.map(f => f.author?.left), counts: facts.map(f => f.count) }));
    };
    await open(source);
    await check("main", page.locator(".page-blocks").first());
    await page.locator('.page-blocks .ls-block[data-block-ref="88300000-0000-4000-8000-000000000001"] > .block-main .bullet-container').first().click();
    await page.locator(".zoomed-page").waitFor();
    await check("zoom", page.locator(".zoomed-block").first());
    await open("Author alignment embeds");
    await page.locator(".embed-block").nth(1).waitFor();
    await check("block-embed", page.locator(".embed-block").nth(0));
    await check("page-embed", page.locator(".embed-block").nth(1));
    await page.evaluate(async name => (await import("/src/ui.ts")).openPageInSidebar(name), source);
    await check("sidebar", page.locator(".right-sidebar"));
    await page.close();
  }
  assert.deepEqual(browserErrors, []);
  assert.deepEqual(failures, []);
  console.log(`GH #883: ${results.length} geometry and reference-toggle journeys passed; screenshots in ${out}`);
} catch (error) {
  await activePage?.screenshot({ path: `${out}/failure.png` }).catch(() => {});
  writeFileSync(`${out}/failure.json`, JSON.stringify({ ...proof(), observed: String(error),
    classification: failures.length ? "product" : "harness", evidence: "proof.json and screenshots" }, null, 2));
  throw error;
} finally {
  writeFileSync(`${out}/proof.json`, JSON.stringify(proof(), null, 2));
  await browser?.close();
  await server.close();
}

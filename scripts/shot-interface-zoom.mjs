// GH #826: the real browser App, CSS mobile zoom and Android resize/IME events.
// No native backend or personal graph. Run sequentially on shared gate hosts.
import { chromium } from "playwright";
import { createServer } from "vite";
import solid from "vite-plugin-solid";
import { mkdirSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import assert from "node:assert/strict";

const out = "target/artifacts/fix-beta-zoom-826";
const label = process.env.ZOOM_PROOF_LABEL ?? "after";
const baseline = process.env.ZOOM_PROOF_BASE;
const baselineFiles = ["src/zoom.ts", "src/components/MobileKeyboardToolbar.tsx", "src/components/RenderedSelectionToolbar.tsx", "src/components/block/selectionToolbarPlacement.ts", "src/styles/app.css", "src/styles/query.css", "src/styles/topbar.css", "src/styles/touchGestures.css", "src/styles/help.css"];
mkdirSync(out, { recursive: true });
writeFileSync(`${out}/query-fixture.tsx`, `
import { render } from "solid-js/web";
import { QueryBuilder } from "/src/components/QueryBuilder";
import { taskFilter } from "/src/editor/queryBuilder";
export function mountQuery() {
  const host = document.createElement("div");
  document.body.append(host);
  const session = { query: { anchor: "block", filter: { kind: "and", items: Array.from({ length: 20 }, () => taskFilter(["TODO"])) }, source: { kind: "builder" } }, view: {} };
  const dispose = render(() => <QueryBuilder session={() => session} onChange={() => {}} />, host);
  host.querySelector(".qs-gear").click();
  return () => { dispose(); host.remove(); };
}
`);
// This journey never opens a PDF; omit the production PDF decoder build hook.
const baselinePlugin = {
  name: "zoom-proof-baseline", enforce: "pre",
  load(id) {
    const relative = id.replace(process.cwd() + "/", "").split("?")[0];
    if (baseline && baselineFiles.includes(relative)) return execFileSync("git", ["show", `${baseline}:${relative}`], { encoding: "utf8", maxBuffer: 4 * 1024 * 1024 });
  },
};
const server = await createServer({ configFile: false, plugins: [baselinePlugin, solid()],
  define: { __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00Z"), __GIT_COMMIT__: JSON.stringify("zoom-proof"), __TINE_COMMUNITY_REGISTRY__: "true" },
  server: { port: 0, fs: { allow: [process.cwd(), process.cwd() + "/node_modules"] } } });
await server.listen();
let browser;
const results = [], failures = [];
try {
  browser = await chromium.launch({ executablePath: process.env.ZOOM_BROWSER_EXECUTABLE, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 390, height: 844 }, hasTouch: true });
  page.setDefaultTimeout(4000);
  page.setDefaultNavigationTimeout(60000);
  page.on("pageerror", error => console.error(String(error)));
  await page.addInitScript(() => {
    globalThis.__TINE_PLATFORM__ = "android";
    localStorage.setItem("logseq-claude.sidebarOpen", "0");
  });
  await page.goto(server.resolvedUrls.local[0]);
  await page.locator("[data-search-trigger]").waitFor({ timeout: 60000 });
  await page.evaluate(async () => {
    (await import("/src/navSettings.ts")).setMobileNavigationAtBottom(true);
    (await import("/src/router.ts")).openPage("Tine");
  });
  await page.locator(".page-title").filter({ hasText: "Tine" }).waitFor();
  while (await page.locator(".toast-close").count()) await page.locator(".toast-close").first().click();
  const check = async (name, run, surface = page) => {
    try { await run(); results.push({ name, pass: true }); console.log(`PASS ${name}`); }
    catch (error) {
      failures.push({ name, error: String(error) });
      console.error(`FAIL ${name}: ${error}`);
      await surface.screenshot({ path: `${out}/${label}-failure-${failures.length}.png` });
    }
  };
  const fits = async (selector, surface = page) => {
    const box = await surface.locator(selector).first().boundingBox();
    const view = surface.viewportSize();
    assert.ok(box && box.width > 0 && box.height > 0 && box.x >= -1 && box.y >= -1
      && box.x + box.width <= view.width + 1 && box.y + box.height <= view.height + 1,
    `${selector} must fit ${JSON.stringify(view)}; observed ${JSON.stringify(box)}`);
    return box;
  };
  for (const scale of [1, 1.1, 1.5, 0.5, 3]) {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.evaluate(async scale => {
      (await import("/src/zoom.ts")).applyAndroidInterfaceZoom(scale);
      document.activeElement?.blur();
      window.dispatchEvent(new CustomEvent("tine-ime", { detail: false }));
    }, scale);
    await page.waitForTimeout(100);
    await page.screenshot({ path: `${out}/${label}-${scale}-closed.png` });
    await check(`${scale}: keyboard closed, navigation fits and Search opens`, async () => {
      const shell = await fits(".app-container");
      assert.ok(Math.abs(shell.y + shell.height - 844) <= 1, "shell reaches the viewport bottom at reduced zoom too");
      const bar = await fits(".mobile-navigation-toolbar");
      assert.ok(Math.abs(bar.y + bar.height - 844) <= 1, "navigation remains at the bottom");
      for (const button of await page.locator(".mobile-navigation-toolbar button").all()) {
        await button.scrollIntoViewIfNeeded();
        const hit = await button.evaluate(el => {
          const r = el.getBoundingClientRect();
          return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
        });
        assert.ok(hit, "each navigation action has a reachable touch target");
      }
      const search = page.locator(".mobile-navigation-toolbar [data-search-trigger]");
      await search.tap({ timeout: 2000 });
      await fits(".switcher");
      await page.keyboard.press("Escape");
    });
    await page.keyboard.press("Escape");
    await check(`${scale}: settings and both sidebars fit`, async () => {
      await page.evaluate(async () => (await import("/src/ui.ts")).openSettings());
      await fits(".settings-modal");
      await page.screenshot({ path: `${out}/${label}-${scale}-settings.png` });
      await page.evaluate(async () => (await import("/src/ui.ts")).closeSettings());
      for (const side of ["Left", "Right"]) {
        await page.evaluate(async side => (await import("/src/ui.ts"))[`set${side}SidebarOpen`](true), side);
        await fits(`[data-mobile-drawer-panel="${side.toLowerCase()}"]`);
        await page.screenshot({ path: `${out}/${label}-${scale}-${side.toLowerCase()}-drawer.png` });
        await page.evaluate(async side => (await import("/src/ui.ts"))[`set${side}SidebarOpen`](false), side);
      }
    });
    await page.evaluate(async () => {
      const ui = await import("/src/ui.ts");
      ui.closeSettings(); ui.setLeftSidebarOpen(false); ui.setRightSidebarOpen(false);
    });
    await check(`${scale}: query bottom sheet stays on screen and scrolls`, async () => {
      await page.evaluate(async () => { window.disposeZoomQuery = (await import("/target/artifacts/fix-beta-zoom-826/query-fixture.tsx")).mountQuery(); });
      await fits(".qs-sheet");
      const scrolls = await page.locator(".qs-sheet").evaluate(el => {
        el.scrollTop = el.scrollHeight;
        return el.scrollTop > 0;
      });
      assert.ok(scrolls, "long query sheets retain scrolling");
      await page.screenshot({ path: `${out}/${label}-${scale}-query-sheet.png` });
    });
    await page.evaluate(() => window.disposeZoomQuery?.());
    await check(`${scale}: keyboard open, editing dock and formatting fit`, async () => {
      await page.locator(".main-content .ls-block .block-content").filter({ hasText: "Reads the same markdown graph", visible: true }).first().tap({ timeout: 2000 });
      const editor = page.locator("textarea.block-editor");
      await editor.waitFor({ timeout: 2000 });
      await page.evaluate(() => window.dispatchEvent(new CustomEvent("tine-ime", { detail: true })));
      await page.setViewportSize({ width: 390, height: 544 });
      await page.waitForTimeout(100);
      await page.screenshot({ path: `${out}/${label}-${scale}-open.png` });
      await fits("[data-mobile-keyboard-toolbar]");
      await editor.evaluate(el => { el.setSelectionRange(0, 4); el.dispatchEvent(new Event("select", { bubbles: true })); });
      await page.waitForTimeout(100);
      await fits("[data-mobile-selection-toolbar]");
      for (const button of await page.locator("[data-mobile-selection-toolbar] > button:visible").all()) {
        assert.ok(await button.evaluate(el => {
          const r = el.getBoundingClientRect();
          return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
        }), "every formatting action has a reachable touch target");
      }
      await page.screenshot({ path: `${out}/${label}-${scale}-formatting.png` });
      const before = await editor.inputValue();
      const reference = page.getByRole("button", { name: "Page reference", exact: true });
      await reference.scrollIntoViewIfNeeded({ timeout: 2000 });
      await reference.tap({ timeout: 2000 });
      assert.ok((await editor.inputValue()).includes("[["), "dock command inserts a page reference");
      const undo = page.getByRole("button", { name: "Undo", exact: true });
      await undo.scrollIntoViewIfNeeded({ timeout: 2000 });
      await undo.tap({ timeout: 2000 });
      assert.equal(await editor.count(), 1, "dock command preserves the editor");
      assert.equal(await editor.inputValue(), before, "Undo restores the original text");
    });
    await page.evaluate(() => {
      window.dispatchEvent(new CustomEvent("tine-ime", { detail: false }));
      document.activeElement?.blur();
    });
  }
  // The setting rounds to whole percentage points and clamps to 50..300%.
  // Cheap geometry coverage includes every such value; touch journeys above
  // cover representative values and both endpoints without 502 repeated taps.
  for (const keyboard of [false, true]) {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.evaluate(async () => (await import("/src/zoom.ts")).applyAndroidInterfaceZoom(1));
    if (keyboard) await page.locator(".main-content .ls-block .block-content").filter({ hasText: "Reads the same markdown graph", visible: true }).first().tap();
    await page.evaluate(detail => window.dispatchEvent(new CustomEvent("tine-ime", { detail })), keyboard);
    if (keyboard) await page.setViewportSize({ width: 390, height: 544 });
    await check(`all 251 zoom percentages: keyboard ${keyboard ? "open" : "closed"}`, async () => {
      const issue = await page.evaluate(async keyboard => {
        const { applyAndroidInterfaceZoom } = await import("/src/zoom.ts");
        for (let percent = 50; percent <= 300; percent++) {
          applyAndroidInterfaceZoom(percent / 100);
          await new Promise(requestAnimationFrame);
          for (const selector of [".app-container", keyboard ? "[data-mobile-keyboard-toolbar]" : ".mobile-navigation-toolbar"]) {
            const r = document.querySelector(selector).getBoundingClientRect();
            if (r.width <= 0 || r.height <= 0 || r.left < -1 || r.right > innerWidth + 1 || r.top < -1 || Math.abs(r.bottom - innerHeight) > 1) {
              return { percent, selector, rect: r.toJSON(), width: innerWidth, height: innerHeight };
            }
          }
        }
        return null;
      }, keyboard);
      assert.equal(issue, null, JSON.stringify(issue));
    });
  }
  await page.close();
  for (const [kind, width, height, touch] of [["tablet", 820, 1180, true], ["desktop", 1280, 900, false]]) {
    const surface = await browser.newPage({ viewport: { width, height }, hasTouch: touch });
    surface.setDefaultTimeout(4000);
    surface.setDefaultNavigationTimeout(60000);
    await surface.addInitScript(kind => { globalThis.__TINE_PLATFORM__ = kind === "tablet" ? "android" : "linux"; localStorage.setItem("logseq-claude.sidebarOpen", "0"); }, kind);
    await surface.goto(server.resolvedUrls.local[0]);
    await surface.locator("[data-search-trigger]").waitFor({ timeout: 60000 });
    for (const dir of ["row", "col"]) {
      const pane = await surface.evaluate(async dir => (await import("/src/panes.ts")).splitPane(undefined, dir), dir);
      assert.ok(pane, `${kind}: split is available`);
      for (const scale of kind === "tablet" ? [1, 1.1, 1.5, 0.5, 3] : [1]) {
        await surface.evaluate(async scale => (await import("/src/zoom.ts")).applyAndroidInterfaceZoom(scale), scale);
        await check(`${kind} ${dir} split ${scale}: panes stay within the viewport`, async () => {
          await fits(".app-container", surface);
          assert.equal(await surface.locator(".pane-leaf").count(), 2);
          for (const leaf of await surface.locator(".pane-leaf").all()) {
            const r = await leaf.boundingBox();
            assert.ok(r && r.width > 0 && r.height > 0 && r.x >= -1 && r.y >= -1 && r.x + r.width <= width + 1 && r.y + r.height <= height + 1, JSON.stringify(r));
          }
          await surface.screenshot({ path: `${out}/${label}-${kind}-${dir}-${scale}.png` });
        }, surface);
      }
      await surface.evaluate(async pane => (await import("/src/panes.ts")).closePane(pane), pane);
    }
    await surface.close();
  }
} finally {
  await browser?.close();
  await server.close();
  writeFileSync(`${out}/${label}-results.json`, JSON.stringify({ commit: baseline ?? execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), baseline: !!baseline, results, failures }, null, 2));
}
console.log(JSON.stringify({ results, failures }, null, 2));
assert.equal(failures.length, 0, "interface zoom journeys must pass");

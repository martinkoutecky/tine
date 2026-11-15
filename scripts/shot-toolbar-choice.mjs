// #926: default equivalence plus the routed-page customization path, synthetic data.
import { chromium } from "playwright";
import { createServer } from "vite";
import solid from "vite-plugin-solid";
import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import assert from "node:assert/strict";

const out = "target/artifacts/toolbar-926";
const baseline = process.env.TOOLBAR_BASE;
mkdirSync(out, { recursive: true });
const oldFiles = new Set(["src/App.tsx", "src/styles/topbar.css", "src/components/TopbarOverflowMenu.tsx", "src/components/CalendarJump.tsx", "src/components/WorkspaceSwitcher.tsx"]);
const server = await createServer({ configFile: false, cacheDir: `${out}/vite-cache-${baseline ? "before" : "after"}`, plugins: [{ name: "toolbar-base", enforce: "pre", load(id) {
  const relative = id.replace(process.cwd() + "/", "").split("?")[0];
  let source;
  if (baseline && oldFiles.has(relative)) source = execFileSync("git", ["-C", process.cwd(), "show", `${baseline}:${relative}`], { encoding: "utf8" });
  if (relative === "src/App.tsx") {
    source ??= readFileSync(relative, "utf8");
    // Exercise the real native-controls branch with the synthetic web backend.
    return source.replaceAll("isTauri() && !osDrawsWindowControls()", '(localStorage.getItem("toolbar-proof-frame") === "1" || (isTauri() && !osDrawsWindowControls()))');
  }
  if (relative === "src/components/WindowChrome.tsx") {
    // Native window actions are not invoked by this geometry proof.
    return readFileSync(relative, "utf8").replaceAll("getCurrentWindow()", "({})");
  }
  return source;
} }, solid()], define: { __BUILD_TIME__: '"1970-01-01T00:00:00Z"', __GIT_COMMIT__: '"toolbar-proof"', __TINE_COMMUNITY_REGISTRY__: "false" },
  server: { port: 0, fs: { allow: [process.cwd(), realpathSync("node_modules"), realpathSync("node_modules/inter-ui")] }, watch: { ignored: ["**/target/**"] } } });
await server.listen();
let browser;
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 390, height: 844 }, hasTouch: true });
  page.on("pageerror", error => console.error(error));
  await page.addInitScript(() => localStorage.setItem("logseq-claude.sidebarOpen", "0"));
  await page.goto(server.resolvedUrls.local[0]);
  await page.locator("[data-search-trigger]").waitFor({ timeout: 60000 });
  await page.waitForLoadState("networkidle");
  await page.evaluate(() => document.fonts.ready);
  await page.evaluate(async () => { (await import("/src/router.ts")).openPage("Tine"); });
  const visible = () => {
    const bar = document.querySelector("header.topbar");
    const rect = bar.getBoundingClientRect();
    const buttons = [...bar.querySelectorAll("button")].filter(button => button.getBoundingClientRect().width && !button.closest('[role="menu"]'));
    return { buttons: buttons.map(button => button.title || button.getAttribute("aria-label") || button.textContent.trim()),
      clipped: buttons.filter(button => button.getBoundingClientRect().right > rect.right + 1 || button.getBoundingClientRect().left < rect.left - 1).map(button => button.title) };
  };
  const results = [];
  for (const width of [900, 488, 460, 440, 420, 414, 400, 390, 360, 332, 328, 300, 280, 250, 240, 222, 194]) {
    await page.setViewportSize({ width, height: 844 });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const result = await page.evaluate(visible);
    assert.deepEqual(result.clipped, [], `default clipping at ${width}`);
    results.push({ width, ...result });
    if ([390, 900].includes(width)) await page.screenshot({ path: `${out}/${baseline ? "before" : "after"}-default-${width}.png` });
  }
  for (const zoom of [1.5, 2]) {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.evaluate(async zoom => { (await import("/src/zoom.ts")).applyAndroidInterfaceZoom(zoom); }, zoom);
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    results.push({ width: 390, zoom, ...await page.evaluate(visible) });
  }
  await page.evaluate(async () => { (await import("/src/zoom.ts")).applyAndroidInterfaceZoom(1); });
  await page.evaluate(() => localStorage.setItem("toolbar-proof-frame", "1"));
  for (const width of [488, 440, 400]) {
    await page.setViewportSize({ width, height: 844 });
    await page.reload();
    await page.locator("[data-search-trigger]").waitFor();
    await page.waitForLoadState("networkidle");
    await page.evaluate(async () => { (await import("/src/router.ts")).openPage("Tine"); });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    results.push({ width, windowControls: true, ...await page.evaluate(visible) });
  }
  await page.evaluate(() => localStorage.setItem("toolbar-proof-frame", "0"));
  await page.reload();
  await page.locator("[data-search-trigger]").waitFor();
  await page.waitForLoadState("networkidle");
  await page.evaluate(async () => { (await import("/src/router.ts")).openPage("Tine"); });
  writeFileSync(`${out}/${baseline ? "before" : "after"}-default.json`, JSON.stringify(results, null, 2));
  if (!baseline) {
    assert.deepEqual(results, JSON.parse(readFileSync(`${out}/before-default.json`, "utf8")), "default bar changed");
    await page.setViewportSize({ width: 390, height: 844 });
    await page.evaluate(async () => { (await import("/src/ui.ts")).openSettings("appearance"); });
    await page.locator('[data-toolbar-section="bar"]').waitFor();
    for (const id of ["calendar", "journals", "share", "theme", "right-sidebar", "workspaces"]) {
      await page.locator(`[data-toolbar-choice="${id}"] button`).last().click();
    }
    await page.screenshot({ path: `${out}/after-settings-390.png` });
    await page.evaluate(async () => { (await import("/src/ui.ts")).closeSettings(); });
    await page.locator('[data-topbar-overflow-trigger]').click();
    await page.getByRole("menuitem", { name: "Customize toolbar…", exact: true }).waitFor();
    const menu = await page.locator('[data-topbar-overflow-action]').evaluateAll(elements => elements.filter(element => element.getBoundingClientRect().width).map(element => element.dataset.topbarOverflowAction));
    assert.deepEqual(menu, ["calendar", "journals", "share", "theme", "right-sidebar", "workspaces"]);
    await page.screenshot({ path: `${out}/after-choice-390.png` });
    await page.locator('[data-topbar-overflow-action="calendar"]').click();
    await page.locator('.calendar-jump-pop').waitFor();
    await page.keyboard.press("Escape");
    await page.locator('[data-topbar-overflow-trigger]').click();
    await page.locator('[data-topbar-overflow-action="workspaces"]').click();
    await page.locator('[data-workspace-switcher-sidebar] .workspace-menu').waitFor();
    assert.equal(await page.locator('.workspace-edit-input').count(), 0, "Workspaces must open the picker without a creation form");
    await page.evaluate(async () => { (await import("/src/ui.ts")).setLeftSidebarOpen(false); });
    await page.locator('[data-topbar-overflow-trigger]').click();
    await page.getByRole("menuitem", { name: "Customize toolbar…", exact: true }).click();
    await page.waitForFunction(() => { const section = document.querySelector('[data-setting-id="toolbar-title"]'); if (!section) return false; const rect = section.getBoundingClientRect(); return rect.top > 0 && rect.top < window.innerHeight; });
    await page.getByRole("button", { name: "Reset toolbar", exact: true }).click();
    await page.evaluate(async () => { (await import("/src/ui.ts")).closeSettings(); });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
    assert.deepEqual(await page.evaluate(visible), results.find(result => result.width === 390) && { buttons: results.find(result => result.width === 390).buttons, clipped: [] });
    // A desktop choice and a width change exercise measured rightmost overflow.
    await page.setViewportSize({ width: 900, height: 844 });
    await page.evaluate(async () => { const preferences = await import("/src/toolbarChoice.ts"); preferences.saveToolbarChoice({ bar: ["settings", "theme", "calendar", "journals", "share", "right-sidebar", "workspaces"], menu: [] }); });
    await page.screenshot({ path: `${out}/after-choice-900.png` });
    await page.locator('[data-toolbar-action="settings"]').focus();
    await page.keyboard.press("Tab");
    assert.equal(await page.evaluate(() => document.activeElement.dataset.toolbarAction), "theme", "Tab follows the chosen toolbar order");
    await page.keyboard.press("Tab");
    assert.equal(await page.evaluate(() => document.activeElement.dataset.toolbarAction), "calendar", "Tab continues through the chosen toolbar order");
    await page.locator('header.topbar [data-workspace-switcher] > button').click();
    await page.locator('header.topbar .workspace-menu').waitFor();
    assert(await page.locator('header.topbar .workspace-menu').evaluate(menu => { const rect = menu.getBoundingClientRect(); return rect.left >= 0 && rect.right <= window.innerWidth; }), "chosen Workspaces menu leaves the screen");
    await page.locator('header.topbar [data-workspace-switcher] > button').click();
    await page.setViewportSize({ width: 900, height: 1200 });
    await page.evaluate(async () => { (await import("/src/ui.ts")).openSettings("appearance"); });
    const source = page.locator('[data-toolbar-choice="settings"] .toolbar-drag-handle');
    const destination = page.locator('[data-toolbar-choice="theme"]');
    await destination.scrollIntoViewIfNeeded();
    const from = await source.boundingBox(), to = await destination.boundingBox();
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(to.x + 20, to.y + to.height - 2, { steps: 8 });
    await page.mouse.up();
    assert.deepEqual(await page.locator('[data-toolbar-section="bar"] li').evaluateAll(rows => rows.slice(0, 2).map(row => row.dataset.toolbarChoice)), ["theme", "settings"]);
    const touchSource = page.locator('[data-toolbar-choice="theme"] .toolbar-drag-handle');
    const touchDestination = page.locator('[data-toolbar-section="menu"]');
    await touchDestination.scrollIntoViewIfNeeded();
    const touchFrom = await touchSource.boundingBox(), touchTo = await touchDestination.boundingBox();
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: touchFrom.x + 8, y: touchFrom.y + 8 }] });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: touchTo.x + 30, y: touchTo.y + touchTo.height / 2 }] });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await page.locator('[data-toolbar-section="menu"] [data-toolbar-choice="theme"]').waitFor();
    await cdp.detach();
    await page.evaluate(async () => { (await import("/src/ui.ts")).closeSettings(); });
    const beforeNarrow = await page.locator('[data-toolbar-action]').evaluateAll(elements => elements.filter(element => element.getBoundingClientRect().width).map(element => element.dataset.toolbarAction));
    await page.setViewportSize({ width: 240, height: 844 });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const afterNarrow = await page.locator('[data-toolbar-action]').evaluateAll(elements => elements.filter(element => element.getBoundingClientRect().width).map(element => element.dataset.toolbarAction));
    assert(afterNarrow.length < beforeNarrow.length, "narrowing did not overflow chosen buttons");
    assert(afterNarrow.includes("settings"), "leftmost chosen action must stay");
    console.log(JSON.stringify({ defaultCases: results.length, menu, beforeNarrow, afterNarrow }));
  } else console.log(`Recorded ${results.length} default cases at ${baseline}`);
} finally { await browser?.close(); await server.close(); }

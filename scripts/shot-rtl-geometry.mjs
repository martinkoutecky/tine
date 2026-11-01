// GH #823: measured physical sides must agree with pointer/keyboard commands.
import { spawn } from "node:child_process";
import { mkdirSync, openSync, closeSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { chromium } from "playwright";

const out = resolve(process.argv[2] || "artifacts/og-rtl-823/fix-geometry");
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const log = openSync(`${out}/vite.log`, "w");
const server = spawn("node", ["node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: ["ignore", log, log], detached: true });
closeSync(log);
let browser;
const facts = [], failures = [];
const check = (ok, label) => { if (!ok) failures.push(label); };
try {
  for (let i = 0; ; i++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (i === 300) throw new Error("Vite did not start");
    await delay(200);
  }
  browser = await chromium.launch({ ...(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {}), args: ["--no-sandbox"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`http://localhost:${port}/`);
  await page.waitForSelector(".ls-block");
  await page.evaluate(async () => { const { setActiveLocale } = await import("/src/i18n/index.ts"); await setActiveLocale("fa"); });
  for (const x of [10, 600, 1260]) {
    await page.evaluate(async x => {
      const { openPageContextMenu } = await import("/src/ui.ts");
      const { node } = await import("/src/document/index.ts");
      openPageContextMenu(x, 160, node(document.querySelector(".ls-block").dataset.blockId).page);
    }, x);
    await page.locator(".ctx-menu").waitFor();
    await page.locator(".ctx-submenu").first().hover();
    const menu = await page.locator(".ctx-menu").evaluate(el => {
      const r = el.querySelector(".ctx-submenu-menu").getBoundingClientRect();
      return { side: el.dataset.submenuSide, left: r.left, right: r.right, width: innerWidth };
    });
    facts.push({ menu, x });
    check(menu.left >= 0 && menu.right <= menu.width, `submenu at ${x} stays on screen`);
    check(menu.side === (x === 10 ? "right" : "left"), `submenu at ${x} prefers the available physical side`);
    await page.screenshot({ path: `${out}/submenu-${x}.png` });
    await page.keyboard.press("Escape");
  }
  // Browser builds have no native chrome. Use the exact production grip classes.
  const grips = await page.evaluate(() => {
    const box = el => { const r = el.getBoundingClientRect(); return { left: r.left, right: r.right }; };
    const els = ["e", "w", "ne", "nw", "se", "sw"].map(side => {
      const el = document.createElement("div"); el.className = `resize-grip grip-${side}`; document.body.append(el); return el;
    });
    const result = els.map(el => ({ side: el.className, ...box(el) })); els.forEach(el => el.remove()); return { result, width: innerWidth };
  });
  facts.push(grips);
  for (const g of grips.result) check(g.side.endsWith("e") ? g.right === grips.width : g.left === 0, `${g.side} is physical`);

  // Use the actual drag handler and split insertion path, with enough tabs to
  // donate one. Compare preview and drop result to the screen's left side.
  await page.evaluate(async () => {
    const { paneRouter } = await import("/src/panes.ts");
    const r = paneRouter("main");
    r.openInNewTab({ kind: "page", name: "RTL split probe" }, true);
  });
  await delay(200);
  const source = page.locator('.tab[data-tab-id]').last();
  const s = await source.boundingBox();
  const pane = await page.locator('[data-pane-id="main"]').boundingBox();
  await page.mouse.move(s.x + s.width / 2, s.y + s.height / 2); await page.mouse.down();
  await page.mouse.move(pane.x + 3, pane.y + pane.height / 2, { steps: 8 });
  await page.locator(".pane-tab-split-preview-left").waitFor();
  const preview = await page.locator(".pane-tab-split-preview-left").boundingBox();
  check(preview.x < pane.x + pane.width / 2, "left drag preview is on the left");
  await page.screenshot({ path: `${out}/split-preview.png` });
  await page.mouse.up();
  await page.waitForFunction(() => document.querySelectorAll("[data-pane-id]").length >= 2);
  const split = await page.evaluate(async () => {
    const { focusedPaneId } = await import("/src/panes.ts");
    return [...document.querySelectorAll("[data-pane-id]")].map(el => ({ id: el.dataset.paneId, x: el.getBoundingClientRect().left, focused: focusedPaneId() === el.dataset.paneId }));
  });
  facts.push({ preview, split });
  check(split.find(p => p.focused).x < split.find(p => p.id === "main").x, "the dropped pane inserts on the left");
  await page.screenshot({ path: `${out}/split-result.png` });

  // Enter pane-select and use a real arrow; its selected edge and Enter result
  // have the same physical side, independent of the interface direction.
  await page.evaluate(async () => {
    const { enterPaneSelect } = await import("/src/paneSelect.ts");
    const { focusedPaneId } = await import("/src/panes.ts");
    enterPaneSelect(focusedPaneId());
  });
  await page.keyboard.press("ArrowLeft");
  const selected = await page.evaluate(async () => (await import("/src/paneSelect.ts")).paneSel());
  check(selected?.side === "left", "keyboard selected physical left edge");
  const edge = await page.locator(".pane-edge-seg-left, .pane-edge-highlight-left").first().boundingBox();
  facts.push({ selected, edge });
  check(edge.x < preview.x + preview.width, "keyboard edge paints left");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Escape");
  await delay(100);
  const keyboardSplit = await page.evaluate(async () => {
    const { focusedPaneId } = await import("/src/panes.ts");
    return [...document.querySelectorAll("[data-pane-id]")].map(el => ({ id: el.dataset.paneId, x: el.getBoundingClientRect().left, focused: focusedPaneId() === el.dataset.paneId }));
  });
  facts.push({ keyboardSplit });
  check(keyboardSplit.find(p => p.focused).x < keyboardSplit.find(p => p.id === selected.paneId).x, "Enter inserts on the keyboard-selected physical left");

  // Tab before/after follows DOM order (RTL strip begins on the right).
  const reorder = await page.evaluate(async () => {
    const { tabDropTargetAt } = await import("/src/components/TabBar.tsx");
    const tab = document.querySelector(".tab[data-tab-id]"), r = tab.getBoundingClientRect();
    return { left: tabDropTargetAt(tab, r.left + 2, r.top + r.height / 2), right: tabDropTargetAt(tab, r.right - 2, r.top + r.height / 2) };
  });
  facts.push({ reorder });
  check(reorder.right.before && !reorder.left.before, "RTL tabs insert before on the physical right");

  const reorderPane = await page.evaluate(async () => {
    const { focusedPaneId, paneRouter, togglePaneMaximize } = await import("/src/panes.ts");
    const id = focusedPaneId(), router = paneRouter(id);
    togglePaneMaximize(id); // keep the entire strip visible for the pointer journey
    router.openInNewTab({ kind: "page", name: "Reorder A" }, true);
    router.openInNewTab({ kind: "page", name: "Reorder B" }, true);
    return id;
  });
  await delay(100);
  const tabs = page.locator(`[data-pane-id="${reorderPane}"] .tab[data-tab-id]`);
  const beforeOrder = await tabs.evaluateAll(els => els.map(el => el.dataset.tabId));
  const first = await tabs.nth(0).boundingBox(), second = await tabs.nth(1).boundingBox();
  await page.mouse.move(first.x + first.width / 2, first.y + first.height / 2); await page.mouse.down();
  await page.mouse.move(second.x + 5, second.y + second.height / 2, { steps: 8 });
  const mark = await tabs.nth(1).evaluate(el => {
    const css = getComputedStyle(el, "::after");
    return { after: el.classList.contains("tab-drop-after"), left: css.left, right: css.right };
  });
  check(mark.after && mark.left === "-3px", "RTL tab after indicator paints on the physical left");
  await page.screenshot({ path: `${out}/tab-reorder.png` });
  await page.mouse.up();
  const afterOrder = await tabs.evaluateAll(els => els.map(el => el.dataset.tabId));
  facts.push({ beforeOrder, afterOrder, mark });
  check(afterOrder[0] === beforeOrder[1] && afterOrder[1] === beforeOrder[0], "dropping on the physical left inserts after in the RTL strip");

  const exported = await page.evaluate(async () => {
    const { exportHtml } = await import("/src/editor/exportHtml.ts");
    const html = exportHtml([{ raw: "یادداشت", children: [{ raw: "English child.", children: [] }] }], { stripLinks: false, removeTags: false, removeEmphasis: false });
    const host = document.createElement("div"); host.dir = "ltr"; host.innerHTML = html; document.body.append(host);
    const dir = getComputedStyle(host.querySelector("li li")).direction; host.remove(); return { html, dir };
  });
  facts.push({ exported }); check(exported.dir === "ltr", "exported English child resets RTL inheritance");

  await page.keyboard.press("t"); await page.keyboard.press("s");
  await page.locator(".theme-switch").waitFor();
  for (let i = 0; i < 3; i++) {
    await page.locator(".theme-switch .theme-opt").nth(i).click(); await delay(200);
    const theme = await page.locator(".theme-switch").evaluate(el => {
      const knob = el.querySelector(".theme-knob").getBoundingClientRect(), selected = el.querySelector('[aria-checked="true"]').getBoundingClientRect();
      return { knob: (knob.left + knob.right) / 2, selected: (selected.left + selected.right) / 2 };
    });
    facts.push({ theme }); check(Math.abs(theme.knob - theme.selected) < 2, "RTL theme knob follows the selected option");
  }
  await page.keyboard.press("Escape");

  await page.goto(`http://localhost:${port}/scripts/fixtures/rtl-notes.html`);
  await page.waitForSelector('[data-block-id="fa"]');
  for (const layout of [false, true]) {
    await page.evaluate(layout => window.setNoteLayoutRtl(layout), layout);
    const nested = await page.evaluate(() => Object.fromEntries(["en-child", "rtl-child"].map(id => [id, getComputedStyle(document.querySelector(`[data-block-id="${id}"] .block-content`)).direction])));
    facts.push({ layout, nested });
    check(nested["en-child"] === "ltr" && nested["rtl-child"] === "rtl", `live mixed descendants under ${layout ? "RTL" : "LTR"} outline`);
    await page.screenshot({ path: `${out}/nested-${layout}.png`, fullPage: true });
  }
  await page.evaluate(async () => {
    const { setRaw } = await import("/src/document/index.ts");
    setRaw("num", "یادداشت\n\n| Field | Another | Third | Fourth | Fifth |\n|---|---|---|---|---|\n| Long_unbroken_sample_value | Long_unbroken_sample_value | Long_unbroken_sample_value | Long_unbroken_sample_value | Long_unbroken_sample_value |");
  });
  await page.locator(".md-table-wrap").waitFor(); await delay(200);
  const table = await page.locator(".md-table-wrap").evaluate(el => {
    const r = el.getBoundingClientRect(), pane = el.closest(".main-content").getBoundingClientRect();
    return { left: r.left, right: r.right, paneLeft: pane.left, paneRight: pane.right };
  });
  facts.push({ table });
  check(table.left >= table.paneLeft && table.right <= table.paneRight, "RTL table bleed stays inside its measured pane");
  await page.screenshot({ path: `${out}/table-bleed.png`, fullPage: true });
  await page.route("https://rtl-fixture.test/pixel.png", route => route.fulfill({ contentType: "image/png", body: Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a+mQAAAAASUVORK5CYII=", "base64") }));
  await page.evaluate(async () => {
    const { setRaw } = await import("/src/document/index.ts");
    setRaw("num", 'یادداشت\n\n![a](https://rtl-fixture.test/pixel.png){:width "50%"}');
  });
  const media = page.locator('[data-block-id="num"] .inline-image-wrap');
  await media.waitFor();
  const beforeMedia = await media.boundingBox();
  await media.hover();
  const grip = await media.locator(".img-resize-grip").boundingBox();
  check(grip.x < beforeMedia.x + beforeMedia.width / 2, "RTL media resize grip is on the outward left edge");
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2); await page.mouse.down();
  await page.mouse.move(grip.x + grip.width / 2 - 40, grip.y + grip.height / 2, { steps: 5 }); await page.mouse.up();
  const afterMedia = await media.boundingBox();
  facts.push({ beforeMedia, afterMedia });
  check(afterMedia.width > beforeMedia.width, "dragging the RTL media grip left enlarges the image");
  await delay(200);
  await page.screenshot({ path: `${out}/media-resize.png`, fullPage: true });

  const safeArea = await page.evaluate(async () => {
    await import("/src/styles/help.css");
    document.documentElement.dir = "rtl";
    document.documentElement.style.setProperty("--system-inset-left", "31px");
    document.documentElement.style.setProperty("--system-inset-right", "7px");
    const host = document.createElement("div"); host.innerHTML = '<div class="toast-stack">probe</div><div class="help-corner">?</div><div class="modal-overlay"></div>';
    document.body.append(host);
    const result = { toast: getComputedStyle(host.children[0]).left, help: getComputedStyle(host.children[1]).left,
      overlayLeft: getComputedStyle(host.children[2]).paddingLeft, overlayRight: getComputedStyle(host.children[2]).paddingRight };
    host.remove(); return result;
  });
  facts.push({ safeArea });
  check(safeArea.toast === "49px" && safeArea.help === "47px" && safeArea.overlayLeft === "39px" && safeArea.overlayRight === "15px", "RTL overlays retain the corresponding physical safe-area inset");
  const popovers = await page.evaluate(async () => {
    await import("/src/styles/query.css");
    const { fitPopoverToViewport } = await import("/src/components/popoverFit.ts");
    const result = [];
    for (const x of [8, innerWidth - 58]) {
      const host = document.createElement("div"); host.style.cssText = `position:fixed;left:${x}px;top:150px;width:50px;height:30px`;
      host.innerHTML = '<div class="qs-menu" style="width:200px;height:60px">probe</div>'; document.body.append(host);
      const menu = host.firstElementChild, stop = fitPopoverToViewport(menu);
      await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
      const rect = menu.getBoundingClientRect(); result.push({ left: rect.left, right: rect.right, width: innerWidth });
      stop(); host.remove();
    }
    return result;
  });
  facts.push({ popovers });
  for (const popover of popovers) check(popover.left >= 0 && popover.right <= popover.width, "RTL query popover stays on screen at either edge");
  writeFileSync(`${out}/results.json`, JSON.stringify({ facts, failures }, null, 2));
} finally { await browser?.close(); try { process.kill(-server.pid, "SIGTERM"); } catch {} }
if (failures.length) throw new Error(failures.join("\n"));
console.log(`rtl-geometry: ok, ${out}`);

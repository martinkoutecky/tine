// GH #619 (D-20261018-Q619): Edit button (Q1), Done / Save-Cancel at the trailing end (Q3), distinct
// groups and a visible root connective (Q4). Real components and CSS over the calm-query harness,
// desktop and 390 px touch. The Q2 route and Q6 page-row properties are covered in component tests and
// scripts/shot-query-results.mjs.
// Usage: node scripts/shot-gh619-query-ux.mjs  (writes screenshots, exits non-zero on a violation)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const out = process.env.OUT ?? "artifacts/queries/gh619-ux";
fs.mkdirSync(out, { recursive: true });
const port = 5341;
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: "ignore" });
const failures = [];
const check = (ok, what) => { console.log(ok ? "ok  " : "FAIL", what); if (!ok) failures.push(what); };
let browser;
try {
  for (let i = 0; i < 150; i++) {
    try { if ((await fetch(`http://localhost:${port}/scripts/harness/calm-query.html`)).ok) break; } catch {}
    await sleep(200);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const [width, touch] of [[1100, false], [390, true]]) {
    const tag = `${width}${touch ? "-touch" : ""}`;
    const context = await browser.newContext({ viewport: { width, height: 900 }, hasTouch: touch, isMobile: touch });
    const page = await context.newPage();
    page.on("pageerror", (error) => console.error(error));
    await page.goto(`http://localhost:${port}/scripts/harness/calm-query.html?filter=nested`);
    await page.locator(".query-count").first().waitFor();
    const block = page.locator(".query-block").first();
    const edit = block.locator(".qc-edit");
    check(await edit.count() === 1, `${tag}: an Edit button sits on the query block`);
    const opacity = () => edit.evaluate((el) => Number(getComputedStyle(el).opacity));
    if (touch) check(await opacity() === 1, `${tag}: the Edit button is visible without hover on a touch screen`);
    else {
      await page.mouse.move(2, 2); await sleep(250);
      check(await opacity() === 0, `${tag}: the Edit button rests hidden`);
      await block.hover(); await sleep(250);
      check(await opacity() === 1, `${tag}: hovering the block shows the Edit button`);
      await edit.focus(); await page.mouse.move(2, 2); await sleep(250);
      check(await opacity() === 1, `${tag}: focus keeps the Edit button visible`);
    }
    await page.screenshot({ path: `${out}/edit-button-${tag}.png` });
    const box = await edit.boundingBox();
    const detail = await block.locator(".qc-actions > summary").boundingBox();
    check(!!box && !!detail && box.x + box.width <= detail.x + 1 && Math.abs(box.y - detail.y) < 12, `${tag}: Edit sits just left of the details menu`);
    if (touch) check(!!box && box.width >= 32 && box.height >= 32, `${tag}: the Edit target is finger sized (${box?.width}x${box?.height})`);

    await edit.click();
    const sheet = block.locator(".qc-panel .qs-sheet");
    await sheet.waitFor();
    const places = await page.evaluate(() => {
      const panel = document.querySelector(".qc-panel").getBoundingClientRect();
      const sheetBox = document.querySelector(".qc-panel .qs-sheet").getBoundingClientRect();
      const done = document.querySelector(".qc-panel .qc-done").getBoundingClientRect();
      const groups = [...document.querySelectorAll(".qs-group")];
      const root = document.querySelector(".qs-root-op");
      return { belowSheet: done.top >= sheetBox.bottom - 1, trailing: panel.right - done.right < 40, groups: groups.length,
        groupBorders: groups.map((g) => getComputedStyle(g).borderLeftWidth), root: root?.textContent ?? null };
    });
    console.log(JSON.stringify(places));
    check(places.belowSheet && places.trailing, `${tag}: Done sits bottom-right under the conditions`);
    check(places.groups >= 2 && places.groupBorders.every((w) => parseFloat(w) >= 3), `${tag}: ${places.groups} groups carry the heavier rail`);
    check(places.root === "any of", `${tag}: a root any-of shows its connective (${places.root})`);
    await page.screenshot({ path: `${out}/builder-groups-${tag}.png`, fullPage: true });

    await block.getByLabel("Query actions").click();
    await page.getByRole("button", { name: "Edit as text" }).click();
    const pane = block.locator(".query-text-pane-input");
    await pane.waitFor();
    const clean = await page.evaluate(() => [...document.querySelectorAll(".query-text-pane-actions button")].map((b) => b.textContent.trim()));
    console.log(JSON.stringify(clean));
    check(clean.length === 2 && clean[1].startsWith("Save"), `${tag}: text pane ends with Save (${clean.join(" | ")})`);
    check(clean[0] === "Done", `${tag}: a clean text pane dismisses with Done`);
    await pane.fill((await pane.inputValue()) + " ");
    const dirty = await page.evaluate(() => [...document.querySelectorAll(".query-text-pane-actions button")].map((b) => b.textContent.trim()));
    check(dirty[0] === "Cancel" && dirty[1].startsWith("Save"), `${tag}: an edited text pane shows Cancel just left of Save (${dirty.join(" | ")})`);
    const order = await page.evaluate(() => { const [a, b] = [...document.querySelectorAll(".query-text-pane-actions button")].map((x) => x.getBoundingClientRect()); return a.right <= b.left && Math.abs(a.top - b.top) < 4; });
    check(order, `${tag}: Cancel and Save share a row, Cancel first`);
    await page.screenshot({ path: `${out}/text-pane-${tag}.png`, fullPage: true });
    await context.close();
  }
  // Q6: page rows under the title carry the page's properties as text (query-results harness).
  for (const width of [1000, 390]) {
    const page = await browser.newPage({ viewport: { width, height: 900 } });
    await page.goto(`http://localhost:${port}/scripts/harness/query-results.html`);
    await page.waitForSelector(".query-page-link");
    const props = await page.locator(".query-page-props").allTextContents();
    check(props.length === 9 && props.every((text) => text.includes("source: Synthetic corpus") && text.includes("tags: gptpro")), `${width}: all 9 page rows show their properties under the title`);
    await page.screenshot({ path: `${out}/page-row-properties-${width}.png`, fullPage: true });
    await page.close();
  }
} finally { await browser?.close(); server.kill(); }
if (failures.length) { console.error(`FAIL ${failures.length}: ${failures.join("; ")}`); process.exit(1); }
console.log("PASS GH #619 query UX");

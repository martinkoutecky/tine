// Headless render smoke test against the mock backend. Catches runtime crashes
// in the live sidebar/query paths (circular-import TDZ, render throws, etc.).
// Usage: node scripts/smoke.mjs   (requires `npm run build:mock` first)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5193;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], {
  stdio: "ignore",
});

const fail = (m) => {
  console.error("SMOKE FAIL:", m);
  server.kill("SIGKILL");
  process.exit(1);
};

async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) {
    try {
      const r = await fetch(url);
      if (r.ok) return;
    } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

const errors = [];
try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch();
  const page = await browser.newPage();
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block, .page-loading", { timeout: 5000 });

  // 0) Typing saves through the browser core (ADR 0081): type a word into a
  //    page, leave the editor, and find the word with the Quick Switcher's
  //    block search, which the core answers from the page file it now holds.
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("Tine");
  await page.locator(".switcher-row:not(.block-result) .switcher-name", { hasText: /^Tine$/ }).first().click();
  const row = page.locator(".main-content .block-content", { hasText: "Reads the same markdown graph" }).first();
  await row.click();
  // Either block editor (CodeMirror by default, or the classic textarea).
  const editor = page.locator("textarea.block-editor, [data-cm6-editor]");
  if (!(await editor.waitFor({ timeout: 3000 }).then(() => true, () => false))) fail("the block editor did not open: the page reads as read-only");
  await page.keyboard.press("End");
  await page.keyboard.type(" smoketyped");
  await page.keyboard.press("Escape");
  await sleep(1500); // past the save debounce
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("smoketyped");
  const hit = page.locator(".switcher-row.block-result", { hasText: "smoketyped" });
  if (!(await hit.first().waitFor({ timeout: 5000 }).then(() => true, () => false))) fail("the typed text is not in the core's block search: the edit was not saved");
  await page.keyboard.press("Escape");
  // A reload reopens the fixture graph (the edit is forgotten) for the checks below.
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block, .page-loading", { timeout: 5000 });
  console.log("TYPED: the edit is saved through the core (block search finds it)");
  await sleep(800); // let the feed + queries render

  // 1) The journals feed rendered some blocks.
  const blocks = await page.locator(".ls-block").count();
  if (blocks === 0) fail("no blocks rendered in the feed");

  // 2) A {{query}} rendered (header present) and its collapse triangle works.
  //    The fixture journal's queries sit below the fold (src/mock/graph/journals/),
  //    and the feed renders blocks near the viewport, so scroll down to them.
  for (let i = 0; i < 30 && !(await page.locator(".query-header").count()); i++) {
    await page.evaluate(() => { document.querySelector(".main-content").scrollTop += 600; });
    await sleep(150);
  }
  const qHeader = await page.locator(".query-header").count();
  if (qHeader === 0) fail("no query block rendered");
  await page.locator(".query-collapse").first().click();
  await sleep(150);

  // 2b) The query builder bar renders on the standalone query block, parsing the
  //     existing DSL into chips, and the add-filter picker works end to end:
  //     "+" -> pick "Scheduled" -> a new chip appears (the DSL was rewritten).
  const qbBar = await page.locator(".qb-bar").count();
  if (qbBar === 0) fail("query builder bar did not render");
  const chipsBefore = await page.locator(".qb-bar .qb-chip").count();
  if (chipsBefore === 0) fail("builder parsed no chips from the existing query DSL");
  await page.locator(".qb-add").first().click();
  await page.waitForSelector(".qb-picker", { timeout: 2000 });
  await page.locator(".qb-picker .qb-menu-item", { hasText: "Scheduled" }).first().click();
  await sleep(250);
  const chipsAfter = await page.locator(".qb-bar .qb-chip").count();
  if (chipsAfter <= chipsBefore) fail(`add-filter did not add a chip (${chipsBefore} -> ${chipsAfter})`);
  const hasScheduled = await page.locator(".qb-bar .qb-chip", { hasText: "scheduled" }).count();
  if (hasScheduled === 0) fail("the added 'scheduled' chip is not present");

  // 3) Shift-click a bullet opens that block LIVE in the right sidebar — i.e.
  //    the editable <Block> (with a .block-content-wrapper + collapse toggle),
  //    NOT the old read-only .ref-block snapshot. That structural difference is
  //    exactly the "live & editable" guarantee. (Actually driving the editor
  //    open needs a real focus and can't be asserted headlessly.)
  await page.locator(".page-blocks .bullet-container").first().click({ modifiers: ["Shift"] });
  await page.waitForSelector(".right-sidebar", { timeout: 3000 });
  await sleep(400);
  const sidebarBlocks = await page.locator(".right-sidebar .ls-block").count();
  if (sidebarBlocks === 0) fail("sidebar opened but rendered no block");
  const editableStruct = await page.locator(".right-sidebar .block-content-wrapper").count();
  const readonlyStruct = await page.locator(".right-sidebar .ref-block").count();
  console.log(`DIAG: sidebar ls-block=${sidebarBlocks} editableWrappers=${editableStruct} readonlyRefBlocks=${readonlyStruct} errs=${errors.length}`);
  if (errors.length) console.log("ERRORS:\n" + errors.join("\n"));
  await page.screenshot({ path: "screenshots/smoke.png", fullPage: true }).catch(() => {});
  if (editableStruct === 0) fail("sidebar rendered a read-only block, not the live editable <Block>");


  await browser.close();
  server.kill("SIGKILL");
  if (errors.length) {
    console.error("SMOKE FAIL: console/page errors:\n" + errors.join("\n"));
    process.exit(1);
  }
  console.log(`SMOKE OK: feed=${blocks} blocks, query rendered+collapsible, builder bar add-filter works, sidebar renders live editable <Block>`);
  process.exit(0);
} catch (e) {
  fail(String(e));
}

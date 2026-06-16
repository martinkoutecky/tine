// Headless render smoke test against the mock backend. Catches runtime crashes
// in the live sidebar/query paths (circular-import TDZ, render throws, etc.).
// Usage: node scripts/smoke.mjs   (requires `npm run build` first)
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
  await sleep(800); // let the feed + queries render

  // 1) The journals feed rendered some blocks.
  const blocks = await page.locator(".ls-block").count();
  if (blocks === 0) fail("no blocks rendered in the feed");

  // 2) A {{query}} rendered (header present) and its collapse triangle works.
  const qHeader = await page.locator(".query-header").count();
  if (qHeader === 0) fail("no query block rendered");
  await page.locator(".query-collapse").first().click();
  await sleep(150);

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
  console.log(`SMOKE OK: feed=${blocks} blocks, query rendered+collapsible, sidebar renders live editable <Block>`);
  process.exit(0);
} catch (e) {
  fail(String(e));
}

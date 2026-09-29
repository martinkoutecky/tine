// GH #369 — useless pane scrollbars: reproduce the reporter's "dashboard"
// (multiple short split panes) in headless Chromium against `vite preview`,
// measure each pane's scroll geometry, and screenshot it. jsdom cannot prove
// CSS layout; this harness is the observation boundary for the regression.
//
// Verdict contract (exit 1 = defect reproduced):
//   * a pane whose real content fits its scroller must have NO vertical
//     scrollbar (scrollable range == 0 and no classic scrollbar track width);
//   * this includes reporter-shaped pages occupying 61–99% of a pane, the band
//     the released v0.6.96 harness accidentally exempted;
//   * a pane whose content overflows must still scroll independently;
//   * while editing a long page, its last block is fully on screen at the end
//     with breathing room below it in [25%, 60%] of the pane's own height —
//     the editing slack must be proportional to the PANE, not the window.
//
// Usage: npm run build && source scripts/env.sh && node scripts/shot-pane-slack.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import path from "node:path";

const PORT = Number(process.env.TINE_PANE_SLACK_PORT ?? 5299);
const OUT_BOARD = "/tmp/shot-pane-slack.png";
const OUT_SCROLLED = "/tmp/shot-pane-slack-long-scrolled.png";

const server = spawn(
  process.execPath,
  [path.resolve("node_modules/vite/bin/vite.js"), "preview", "--host", "127.0.0.1", "--port", String(PORT), "--strictPort", "--configLoader", "runner"],
  { stdio: "ignore" },
);

async function waitForServer(url, tries = 80) {
  for (let i = 0; i < tries; i++) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {
      // not up yet
    }
    await sleep(250);
  }
  throw new Error("server did not start");
}

async function openPageInFocusedPane(page, name) {
  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 4000 });
  await page.locator(".switcher-input").fill(name);
  await sleep(400);
  // Enter, not a row click: the switcher overlay sits over another pane, and a
  // pointerdown there would re-target pane focus to the pane beneath it (the
  // capture-phase pane-focus rule), opening the page in the WRONG quadrant.
  await page.keyboard.press("Enter");
  await sleep(250);
}

async function makeFocusedPageReporterHeight(page, lineCount) {
  const splitRoot = page.locator(".pane-leaf.pane-focused");
  const root = (await splitRoot.count()) > 0 ? splitRoot : page.locator(".main-content-shell");
  let editor = root.locator(".block-editor").first();
  if ((await editor.count()) === 0) {
    await root.locator(".ls-block").first().click();
    editor = root.locator(".block-editor").first();
  }
  await editor.waitFor({ timeout: 4000 });
  await editor.fill(Array.from({ length: lineCount }, (_, i) => `dashboard line ${i + 1}`).join("\n"));
  await page.keyboard.press("Escape");
  await sleep(150);
}

async function measurePanes(page) {
  return page.evaluate(() => {
    const out = [];
    for (const leaf of document.querySelectorAll(".pane-leaf")) {
      const scroller = leaf.querySelector(".main-content");
      if (!scroller) continue;
      scroller.scrollTop = 0;
      const inner = scroller.querySelector(".main-content-inner");
      const sr = scroller.getBoundingClientRect();
      let blockBottom = 0;
      for (const block of leaf.querySelectorAll(".ls-block")) {
        const br = block.getBoundingClientRect();
        if (br.bottom > blockBottom) blockBottom = br.bottom;
      }
      out.push({
        title:
          leaf.querySelector(".tab-bar .tab.active .tab-title")?.textContent?.trim() ??
          leaf.querySelector(".tab-bar .tab-title")?.textContent?.trim() ??
          "?",
        rect: { x: sr.x, y: sr.y, w: sr.width, h: sr.height },
        clientHeight: scroller.clientHeight,
        scrollHeight: scroller.scrollHeight,
        range: scroller.scrollHeight - scroller.clientHeight,
        // A visible classic scrollbar consumes layout width on this axis.
        scrollbarW: scroller.offsetWidth - scroller.clientWidth,
        // The full page column (title, blocks, reference panels) at natural
        // height WITHOUT its bottom slack: pre-fix the slack was the inner's
        // own vh bottom padding (counted in scrollHeight); post-fix it lives
        // in the scroller as a sibling spacer. Subtracting the computed bottom
        // padding makes "content" comparable on both sides of the fix.
        slackPad: inner ? parseFloat(getComputedStyle(inner).paddingBottom) || 0 : 0,
        contentH: inner ? inner.scrollHeight - (parseFloat(getComputedStyle(inner).paddingBottom) || 0) : 0,
        blockEnd: blockBottom - sr.top,
      });
    }
    return out;
  });
}

async function measureEndSlack(page, leafRect) {
  return page.evaluate((want) => {
    for (const leaf of document.querySelectorAll(".pane-leaf")) {
      const scroller = leaf.querySelector(".main-content");
      if (!scroller) continue;
      const sr = scroller.getBoundingClientRect();
      if (Math.abs(sr.x - want.x) > 2 || Math.abs(sr.y - want.y) > 2) continue;
      const inner = scroller.querySelector(".main-content-inner");
      let last = null;
      let maxBottom = -Infinity;
      for (const block of leaf.querySelectorAll(".ls-block")) {
        const br = block.getBoundingClientRect();
        if (br.bottom > maxBottom) {
          maxBottom = br.bottom;
          last = br;
        }
      }
      if (!last || !inner) return null;
      const innerRect = inner.getBoundingClientRect();
      return {
        lastBlockBottomVisible: last.bottom <= sr.bottom + 1 && last.bottom >= sr.top,
        // Space between the page column's own end and the pane bottom — this is
        // the editing slack itself, not whatever follows the last block.
        slackH: sr.bottom - innerRect.bottom,
        clientHeight: scroller.clientHeight,
      };
    }
    return null;
  }, leafRect);
}

async function measureFocusedPageWidth(page) {
  return page.evaluate(() => {
    const scroller = document.querySelector(".pane-leaf.pane-focused .main-content") ??
      document.querySelector(".main-content");
    const inner = scroller?.querySelector(".main-content-inner");
    if (!scroller || !inner) return null;
    return {
      scroller: scroller.getBoundingClientRect().width,
      inner: inner.getBoundingClientRect().width,
      maxWidth: getComputedStyle(inner).maxWidth,
    };
  });
}

try {
  await waitForServer(`http://127.0.0.1:${PORT}/`);
  const browser = await chromium.launch({
    args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"],
  });
  // Reporter's window (GH #369 screenshot): 1920x1050.
  const page = await browser.newPage({
    viewport: { width: 1920, height: 1050 },
    deviceScaleFactor: 1,
  });
  const errors = [];
  const failures = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));

  await page.goto(`http://127.0.0.1:${PORT}/`);
  // NOTE on evidence substitution: on the reporter's platform (Windows) and on
  // WebKitGTK (Tauri's Linux engine) scrollbars are classic — they are painted
  // whenever the scrollable range is non-zero. This sandbox's headless
  // Chromium forces auto-hiding OVERLAY bars (probe: range 400px, bar width
  // 0px — bar width cannot be forced classic by flag or CSS), so the contract
  // below asserts the cross-platform quantity that forces a painted bar on the
  // shipped platforms: scrollHeight > clientHeight while content fits.
  await page.waitForSelector(".ls-block", { timeout: 8000 });

  // Open a short page first so the journals feed stays the single-feed-pane
  // singleton out of the way, then build the reporter's 2x2 dashboard.
  await openPageInFocusedPane(page, "Dashboard Solo");
  await page.waitForFunction(
    () => document.querySelector(".main-content .page-title")?.textContent?.trim() === "Dashboard Solo",
    { timeout: 5000 },
  );

  // GH #382: the page column itself must not shrink-wrap to whichever child is
  // currently widest. The reporter showed both edit entry and expanding
  // Unlinked References changing the whole page width. Exercise the intrinsic
  // width boundary directly before building the separate GH #369 pane board.
  const widthStates = [];
  widthStates.push({ state: "short-read", ...(await measureFocusedPageWidth(page)) });
  const widthEditor = page.locator(".main-content .ls-block").first();
  await widthEditor.click();
  const widthTextarea = page.locator(".main-content .block-editor").first();
  await widthTextarea.waitFor({ timeout: 4000 });
  await widthTextarea.fill("x");
  await sleep(100);
  widthStates.push({ state: "short-edit", ...(await measureFocusedPageWidth(page)) });
  await widthTextarea.fill("a long line ".repeat(80));
  await sleep(100);
  widthStates.push({ state: "long-edit", ...(await measureFocusedPageWidth(page)) });
  await page.keyboard.press("Escape");
  await sleep(100);
  widthStates.push({ state: "long-read", ...(await measureFocusedPageWidth(page)) });
  const measuredWidths = widthStates.map((state) => state.inner).filter(Number.isFinite);
  console.log("page width through read/edit content changes:", widthStates);
  if (measuredWidths.length !== widthStates.length) {
    failures.push(`could not measure every GH #382 width state: ${JSON.stringify(widthStates)}`);
  } else if (Math.max(...measuredWidths) - Math.min(...measuredWidths) > 1) {
    failures.push(`page column changes width with its contents: ${JSON.stringify(widthStates)}`);
  }

  await makeFocusedPageReporterHeight(page, 20);
  const solo = await page.evaluate(() => {
    const scroller = document.querySelector(".main-content");
    const inner = scroller?.querySelector(".main-content-inner");
    if (!scroller || !inner) return null;
    const slackPad = parseFloat(getComputedStyle(inner).paddingBottom) || 0;
    return {
      clientHeight: scroller.clientHeight,
      contentH: inner.scrollHeight - slackPad,
      range: scroller.scrollHeight - scroller.clientHeight,
    };
  });
  if (!solo) {
    failures.push("solo pane could not be measured");
  } else {
    const occupancy = solo.contentH / solo.clientHeight;
    console.log(
      `solo dashboard column ${solo.contentH}px of ${solo.clientHeight}px (${Math.round(occupancy * 100)}%) range ${solo.range}px`,
    );
    if (occupancy <= 0.6 || occupancy > 0.99) {
      failures.push(`solo fixture missed the 61–99% reporter band: ${JSON.stringify(solo)}`);
    } else if (solo.range !== 0) {
      failures.push(`fitting idle solo pane still scrolls: ${JSON.stringify(solo)}`);
    }
  }
  await page.keyboard.press("Control+Alt+\\"); // split right
  await page.waitForFunction(() => document.querySelectorAll(".pane-leaf").length === 2, { timeout: 4000 });
  await page.keyboard.press("Control+Alt+Shift+\\"); // split the focused (right) pane down
  await page.waitForFunction(() => document.querySelectorAll(".pane-leaf").length === 3, { timeout: 4000 });
  // Split the remaining full-height pane down too: it is the tallest leaf.
  const tallestIndex = await page.evaluate(() => {
    const leaves = Array.from(document.querySelectorAll(".pane-leaf"));
    let best = 0;
    leaves.forEach((leaf, i) => {
      if (leaf.getBoundingClientRect().height > leaves[best].getBoundingClientRect().height) best = i;
    });
    return best;
  });
  await page.locator(".pane-leaf").nth(tallestIndex).locator(".tab-bar").click({ position: { x: 8, y: 12 } });
  await sleep(250);
  await page.keyboard.press("Control+Alt+Shift+\\");
  await page.waitForFunction(() => document.querySelectorAll(".pane-leaf").length === 4, { timeout: 4000 });

  // Collect the quadrant rectangles once, then reach each one by closed-loop
  // focus: try the documented Ctrl+1..9 pane-focus digits until the pane that
  // carries the focus ring IS the target quadrant. Closed loop because digit
  // reading order is the app's contract, not the harness's assumption.
  const leavesGeom = await page.evaluate(() =>
    Array.from(document.querySelectorAll(".pane-leaf")).map((leaf) => {
      const r = leaf.getBoundingClientRect();
      return { x: r.x, y: r.y, cx: r.x + r.width / 2, cy: r.y + r.height / 2 };
    }),
  );
  const quads = [...leavesGeom].sort((a, b) => a.cy - b.cy || a.cx - b.cx);
  const [TLr, TRr, BLr, BRr] = quads;
  async function focusQuadrant(want) {
    for (let attempt = 0; attempt < 8; attempt++) {
      const cur = await page.evaluate(() => {
        const leaf = document.querySelector(".pane-leaf.pane-focused");
        if (!leaf) return null;
        const r = leaf.getBoundingClientRect();
        return { x: r.x, y: r.y };
      });
      if (cur && Math.abs(cur.x - want.x) <= 2 && Math.abs(cur.y - want.y) <= 2) return;
      const digit = 1 + (attempt % 4);
      await page.keyboard.press(`Control+${digit}`);
      await sleep(180);
    }
    throw new Error(`could not focus quadrant at (${want.x}, ${want.y}) via Ctrl+1..4`);
  }
  // TL keeps "Dashboard Solo" from the initial mirror splits (a page in the
  // reporter's fitting band when solo and a real overflow after the vertical
  // split). TR/BL get
  // fresh one-block pages named after the reporter's own panes: those are the
  // short dashboard cards whose scrollbar must vanish. BR is the long pane
  // that must keep scrolling independently with breathing room at the end.
  const targets = [
    // A single raw multiline block gives these pages natural columns around
    // 70–85% of their half-height panes: the exact fitting band visible in the
    // reporter's Windows dashboard and absent from the original repro.
    { quad: "TR", rect: TRr, name: "Lines", lines: 6 },
    { quad: "BL", rect: BLr, name: "GRID", lines: 8 },
    { quad: "BR", rect: BRr, name: "kitchen-sink" }, // og mock: the long pane
  ];
  for (const t of targets) {
    await focusQuadrant(t.rect);
    await openPageInFocusedPane(page, t.name);
    await page.waitForFunction(
      (name) =>
        document.querySelector(".pane-leaf.pane-focused .tab.active .tab-title")?.textContent?.trim() === name,
      t.name,
      { timeout: 5000 },
    );
    if (t.lines) await makeFocusedPageReporterHeight(page, t.lines);
  }
  await sleep(600);
  await page.screenshot({ path: OUT_BOARD });

  const panes = await measurePanes(page);
  console.log("pane geometry (viewport 1920x1050):");
  for (const p of panes) {
    const P = p.clientHeight;
    const fitsExactly = p.contentH <= P;
    const reporterBand = p.contentH > 0.6 * P && fitsExactly;
    const klass = reporterBand ? "fits reporter band" : fitsExactly ? "fits" : "overflows";
    console.log(
      `  ${p.title.padEnd(16)} pane ${Math.round(p.rect.w)}x${Math.round(p.rect.h)} @(${Math.round(p.rect.x)},${Math.round(p.rect.y)})` +
        `  column ${p.contentH}px of ${P}px (${Math.round((100 * p.contentH) / P)}%)  range ${p.range}px  scrollbarW ${p.scrollbarW}px  ${klass}`,
    );
    // Idle dashboard contract: if the natural page column fits at all, its
    // range is exactly zero. Editing slack is tested separately below.
    const expectedIdle = Math.max(0, p.contentH - P) + (p.contentH > P ? 0.4 * P : 0);
    if (fitsExactly && (p.range > 0 || p.scrollbarW > 0)) {
      failures.push(
        `${p.title}: fitting idle column (${p.contentH}px <= ${P}px) still scrolls ` +
          `(range ${p.range}px, scrollbarW ${p.scrollbarW}px) — the useless GH #369 scrollbar`,
      );
    }
    if (!fitsExactly && p.range <= 0) failures.push(`${p.title}: content overflows (${p.contentH}px > ${P}px) but the pane does not scroll`);
    if (Math.abs(p.range - expectedIdle) > 3) {
      failures.push(
        `${p.title}: idle range ${p.range}px does not match natural overflow (column ${p.contentH}px − pane ${P}px = ${expectedIdle}px)`,
      );
    }
  }

  const reporterBandPanes = panes.filter((p) => p.contentH > 0.6 * p.clientHeight && p.contentH <= 0.99 * p.clientHeight);
  if (reporterBandPanes.length < 2) {
    failures.push(`fixture did not create two 61–99%-full panes: ${JSON.stringify(reporterBandPanes)}`);
  }

  // Long pane: must still scroll independently, end block stays reachable, and
  // proof — proportional to the pane, not the window.
  const longPane = panes.reduce((best, p) => (p.contentH > (best?.contentH ?? 0) ? p : best), null);
  if (!longPane) {
    failures.push("no overflowing pane found — the kitchen-sink (long) pane did not overflow");
  } else {
    // End slack is an editing affordance, not permanent dashboard overflow.
    // Activate the final block before measuring that separate contract.
    // A prior Escape used to finish the synthetic multiline edit may leave
    // block-selection owning digit keys, so use the literal pane tab boundary.
    await page.mouse.click(longPane.rect.x + 18, longPane.rect.y - 16);
    await page.waitForFunction(
      (want) => {
        const scroller = document.querySelector(".pane-leaf.pane-focused .main-content");
        if (!scroller) return false;
        const r = scroller.getBoundingClientRect();
        return Math.abs(r.x - want.x) <= 2 && Math.abs(r.y - want.y) <= 2;
      },
      longPane.rect,
      { timeout: 4000 },
    );

    // GH #390: entering/leaving edit on an already-overflowing long page must
    // not manufacture or remove a pane-relative spacer. Measure the literal
    // scroll range around the transition; this catches the released `:has`
    // rule that grew every long page by 40% on click and shrank it on Escape.
    const editStability = [];
    const measureFocusedScrollHeight = () => page.evaluate(() => {
      const scroller = document.querySelector(".pane-leaf.pane-focused .main-content") ??
        document.querySelector(".main-content");
      return scroller?.scrollHeight ?? null;
    });
    // og mock: a one-line plain-text kitchen-sink block, so only slack (not the
    // raw-vs-rendered form of rich content) could change the height.
    const stableTarget = page.locator(".pane-leaf.pane-focused .ls-block").nth(17);
    // Scroll first and let lazily sized content near the target (og mock's
    // video/audio/embeds) settle, so "read" measures the same content as "edit".
    await stableTarget.scrollIntoViewIfNeeded();
    await sleep(500);
    editStability.push({ state: "read", height: await measureFocusedScrollHeight() });
    await stableTarget.locator(".block-content").first().click();
    await page.locator(".pane-leaf.pane-focused .block-editor").waitFor({ timeout: 4000 });
    await sleep(100);
    editStability.push({ state: "edit", height: await measureFocusedScrollHeight() });
    await page.keyboard.press("Escape");
    await sleep(100);
    editStability.push({ state: "read-again", height: await measureFocusedScrollHeight() });
    const stableHeights = editStability.map((entry) => entry.height).filter(Number.isFinite);
    console.log("long-page height through read/edit transitions:", editStability);
    if (stableHeights.length !== editStability.length || Math.max(...stableHeights) - Math.min(...stableHeights) > 1) {
      failures.push(`long page changes height when a block enters/leaves edit: ${JSON.stringify(editStability)}`);
    }

    await page.locator(".pane-leaf.pane-focused .ls-block").last().click();
    await page.locator(".pane-leaf.pane-focused .block-editor").waitFor({ timeout: 4000 });
    await sleep(150);
    await page.evaluate((want) => {
      for (const leaf of document.querySelectorAll(".pane-leaf")) {
        const scroller = leaf.querySelector(".main-content");
        if (!scroller) continue;
        const sr = scroller.getBoundingClientRect();
        if (Math.abs(sr.x - want.x) <= 2 && Math.abs(sr.y - want.y) <= 2) {
          scroller.scrollTop = scroller.scrollHeight;
        }
      }
    }, longPane.rect);
    await sleep(500);
    await page.screenshot({ path: OUT_SCROLLED });
    const end = await measureEndSlack(page, longPane.rect);
    if (!end) failures.push("could not measure the long pane's end-of-page slack");
    else {
      const slackPct = Math.round((100 * end.slackH) / end.clientHeight);
      console.log(
        `  ${longPane.title}: scrolled to end — last block visible: ${end.lastBlockBottomVisible}, end slack ${Math.round(end.slackH)}px (${slackPct}% of pane)`,
      );
      if (!end.lastBlockBottomVisible) failures.push(`${longPane.title}: last block is NOT reachable/visible at max scroll`);
      if (end.slackH < 0.25 * end.clientHeight || end.slackH > 0.6 * end.clientHeight) {
        failures.push(
          `${longPane.title}: end-of-page editing slack ${slackPct}% of pane height — expected between 25% and 60% (pane-proportional)`,
        );
      }
    }
  }

  if (errors.length) console.log("console errors:\n" + errors.join("\n"));
  console.log(`wrote ${OUT_BOARD} and ${OUT_SCROLLED}`);
  await browser.close();
  server.kill("SIGKILL");
  if (failures.length) {
    console.error("Pane geometry defect REPRODUCED (GH #369 / GH #382):\n - " + failures.join("\n - "));
    process.exit(1);
  }
  console.log("OK: page width is content-independent; short panes have no useless scrollbar; long pane keeps pane-relative end slack");
  process.exit(errors.length ? 1 : 0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}

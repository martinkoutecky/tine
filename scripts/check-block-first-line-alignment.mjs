// Vertical alignment of everything that must sit on a block's FIRST line:
// the bullet dot / ordered number (GH #459) and the reference-count badge
// (GH #454).
//
// jsdom applies no CSS layout, so neither defect is visible to a render test:
// both are line-box arithmetic. This drives the real frontend in headless
// Chromium and measures the rendered boxes.
//
// The oracle is the first line box of `.block-content`, derived from the
// element's own computed style (padding-top + line-height / 2), never from a
// hard-coded pixel. A typography theme that changes the line height therefore
// changes the oracle too, which is the point: the bullet column must follow the
// text, not cancel one particular error with one particular constant.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";

const port = 5203;
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const shots = path.join(root, "screenshots");
mkdirSync(shots, { recursive: true });

// The kitchen-sink block that the mock graph references twice, so it carries a
// count badge (see scripts/shot-blockrefs.mjs).
const REFERENCED_BLOCK = "64b9c0e2-0000-0000-0000-000000000000";
// Half a device pixel: the bullet must land ON the line, not near it.
const TOLERANCE_PX = 0.5;
// A heading keeps its own taller bullet column. This guard only asserts the dot
// is somewhere on the heading's own first line — it deliberately does not freeze
// the heading column's exact height, which is presentation.
const HEADING_TOLERANCE_PX = 3;

async function waitForServer(url) {
  for (let i = 0; i < 80; i += 1) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("preview server did not start");
}

const server = spawn(
  path.join(root, "node_modules", ".bin", "vite"),
  ["preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
  { cwd: root, stdio: "ignore" },
);


/** In-page: the signed distance from the first-line-box centre of every visible
 *  block to the centre of its bullet. Blocks whose content is not ordinary text
 *  flow (macro hosts, and blocks that open with a block-level box such as a code
 *  fence or a table) have no "first line" in this sense and are skipped. */
const measureBullets = () => {
  const rows = [];
  for (const block of document.querySelectorAll(".ls-block")) {
    const main = block.querySelector(":scope > .block-main");
    if (!main) continue;
    const content = main.querySelector(":scope > .block-content-wrapper > .block-content");
    const dot = main.querySelector(":scope > .block-controls .bullet, :scope > .block-controls .bullet-order");
    if (!content || !dot || content.classList.contains("macro-host")) continue;
    // The reference-count badge is a float and opens the content (GH #454); it
    // is not what the first line is made of, so step over it.
    const first = [...content.children].find((child) => !child.classList.contains("block-refs-count")) ?? null;
    if (first && getComputedStyle(first).display !== "inline") continue;
    const style = getComputedStyle(content);
    // The first line box is the taller of the block's own strut and the inline
    // box that opens it — which is how a heading gets a taller first line while
    // h4-h6, whose glyphs are smaller than the strut, keep the ordinary one.
    let lineHeight = parseFloat(style.lineHeight);
    if (first && first.classList.contains("heading-text")) {
      lineHeight = Math.max(lineHeight, parseFloat(getComputedStyle(first).lineHeight));
    }
    if (!Number.isFinite(lineHeight)) continue;
    const box = content.getBoundingClientRect();
    const dotBox = dot.getBoundingClientRect();
    const heading = /\bbullet-h[1-6]\b/.exec(main.className)?.[0] ?? null;
    rows.push({
      heading,
      text: (content.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 40),
      // Distance from the centre of the first line box to the centre of the dot.
      delta: (dotBox.top + dotBox.height / 2) - (box.top + parseFloat(style.paddingTop) + lineHeight / 2),
      contentHeight: box.height,
    });
  }
  return rows;
};

/** In-page: where the reference-count badge sits relative to the first line of
 *  the block it belongs to. */
const measureBadge = (id) => {
  const block = document.querySelector(`.ls-block[data-block-id="${id}"]`);
  const content = block?.querySelector(":scope > .block-main > .block-content-wrapper > .block-content");
  const badge = block?.querySelector(":scope > .block-main > .block-content-wrapper > .block-content > .block-refs-count");
  if (!content || !badge) return { found: false };
  const style = getComputedStyle(content);
  const box = content.getBoundingClientRect();
  const badgeBox = badge.getBoundingClientRect();
  const lineHeight = parseFloat(style.lineHeight);
  return {
    found: true,
    lines: Math.round((box.height - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom)) / lineHeight),
    // 0 when the badge's top edge is level with the top of the first line box.
    offsetFromFirstLine: badgeBox.top - (box.top + parseFloat(style.paddingTop)),
    lineHeight,
  };
};

const problems = [];
const note = (message) => problems.push(message);

try {
  const url = `http://127.0.0.1:${port}/`;
  await waitForServer(url);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1000, height: 1000 } });
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(String(error)));
  await page.goto(url);
  await page.waitForSelector(".ls-block", { timeout: 10000 });
  await sleep(400);

  // ---- GH #459: the bullet holds a column on the first line of its text ----
  const check = (label, rows) => {
    if (rows.length < 5) note(`${label}: only ${rows.length} measurable blocks — the fixture did not render`);
    for (const row of rows) {
      const limit = row.heading ? HEADING_TOLERANCE_PX : TOLERANCE_PX;
      if (Math.abs(row.delta) > limit) {
        note(`${label}: bullet is ${row.delta.toFixed(2)}px off the first line centre (limit ${limit}) on ${row.heading ?? "plain"} block ${JSON.stringify(row.text)}`);
      }
    }
  };
  check("journals", await page.evaluate(measureBullets));
  // The first journal of the day is usually empty; photograph a section that has
  // enough bullets to read as a column.
  const populated = page.locator(".page-section").filter({ has: page.locator(".ls-block >> nth=4") }).first();
  await populated.screenshot({ path: path.join(shots, "block-first-line-journals.png") });

  // The same column under a typography theme that changes the line box. A fix
  // that cancels today's error with a constant fails here.
  await page.evaluate(() => document.documentElement.setAttribute("data-theme-content-typography", "editorial-serif"));
  await sleep(250);
  check("journals/editorial-serif", await page.evaluate(measureBullets));
  await page.evaluate(() => document.documentElement.removeAttribute("data-theme-content-typography"));
  await sleep(250);

  // ---- GH #454: the reference-count badge sits on the first line ----
  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 5000 });
  await page.locator(".switcher-input").fill("kitchen");
  await sleep(500);
  await page.locator(".switcher-row").first().click();
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  await sleep(700);
  await page.locator(`.ls-block[data-block-id="${REFERENCED_BLOCK}"]`).scrollIntoViewIfNeeded();

  const single = await page.evaluate(measureBadge, REFERENCED_BLOCK);
  if (!single.found) note("kitchen-sink: the referenced block has no count badge — fixture drifted");
  else {
    if (single.lines !== 1) note(`kitchen-sink: expected the wide layout to keep the referenced block on one line, saw ${single.lines}`);
    if (Math.abs(single.offsetFromFirstLine) > 1) note(`single-line badge is ${single.offsetFromFirstLine.toFixed(2)}px off its first line`);
  }

  // The kitchen sink carries the shapes the journal feed has not: ordered
  // blocks, headings from macro expansion, collapsed parents.
  check("kitchen-sink", await page.evaluate(measureBullets));

  // Narrow the text column until the same block wraps: the reported case.
  // Constraining the column rather than the window keeps every responsive
  // breakpoint out of the measurement.
  await page.addStyleTag({ content: ":root { --ls-main-content-max-width: 380px; }" });
  await sleep(400);
  await page.locator(`.ls-block[data-block-id="${REFERENCED_BLOCK}"]`).scrollIntoViewIfNeeded();
  await sleep(200);
  const wrapped = await page.evaluate(measureBadge, REFERENCED_BLOCK);
  await page.locator(`.ls-block[data-block-id="${REFERENCED_BLOCK}"]`)
    .screenshot({ path: path.join(shots, "block-refs-badge-wrapped.png") });
  if (!wrapped.found) note("kitchen-sink (narrow): the referenced block has no count badge");
  else {
    if (wrapped.lines < 2) note(`kitchen-sink (narrow): the referenced block did not wrap (${wrapped.lines} line) — the fixture cannot show the defect`);
    if (Math.abs(wrapped.offsetFromFirstLine) > 1) {
      note(`wrapped badge is ${wrapped.offsetFromFirstLine.toFixed(2)}px below its first line`
        + ` (one line is ${wrapped.lineHeight}px — the badge is riding the last line, GH #454)`);
    }
  }

  // ---- headings keep their own, taller bullet column ----
  // Neither demo page renders a heading block with its own bullet column, so the
  // shared-column guard uses the markup Block.tsx emits for one. This exists to
  // catch a change to the ordinary column that drags the heading levels with it;
  // it deliberately does not freeze the heading column's exact height.
  const headingFixture = ["h1", "h2", "h3", "h4"].map((level) => `
    <div class="ls-block">
      <div class="block-main ${["h1", "h2", "h3"].includes(level) ? `bullet-${level}` : ""}">
        <div class="block-controls">
          <span class="collapse-toggle"></span>
          <span class="bullet-container"><span class="bullet"></span></span>
        </div>
        <div class="block-content-wrapper">
          <div class="block-content heading ${level}"><span class="heading-text ${level}">Heading ${level}</span></div>
        </div>
      </div>
    </div>`).join("");
  await page.evaluate((html) => {
    document.body.innerHTML = `<main class="page-section" style="width:520px;padding:0 12px">${html}</main>`;
  }, headingFixture);
  const headingRows = await page.evaluate(measureBullets);
  if (headingRows.length !== 4) note(`heading fixture: expected 4 measurable rows, saw ${headingRows.length}`);
  for (const row of headingRows) {
    const limit = row.heading ? HEADING_TOLERANCE_PX : TOLERANCE_PX;
    if (Math.abs(row.delta) > limit) {
      note(`heading fixture: bullet is ${row.delta.toFixed(2)}px off the first line centre (limit ${limit}) on ${JSON.stringify(row.text)}`);
    }
  }

  if (pageErrors.length) console.log(`(page errors seen, not asserted: ${pageErrors.join(" | ")})`);
  await browser.close();
} finally {
  server.kill("SIGTERM");
}

if (problems.length) {
  console.error(`block first-line alignment FAILED (${problems.length}):`);
  for (const problem of problems) console.error(`  ${problem}`);
  process.exit(1);
}
console.log("block first-line alignment OK: bullets and reference badges sit on their block's first line.");
console.log(`  screenshots/block-first-line-journals.png`);
console.log(`  screenshots/block-refs-badge-wrapped.png`);

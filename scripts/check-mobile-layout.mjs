import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = ["theme.css", "app.css"]
  .map((file) => fs.readFileSync(path.join(root, "src/styles", file), "utf8"))
  .join("\n");

function block({ id, label, children = "", mainClass = "", blockClass = "" }) {
  const foldable = Boolean(children);
  return `
    <div class="ls-block ${blockClass}" data-row="${id}">
      <div class="block-main ${mainClass}">
        <div class="block-controls">
          <span class="collapse-toggle${foldable ? " has-children" : ""}" data-control="${id}"><svg class="triangle"></svg></span>
          <span class="bullet-container" data-bullet="${id}"><span class="bullet"></span></span>
        </div>
        <div class="block-content-wrapper"><div class="block-content" data-content="${id}">${label}</div></div>
      </div>
      ${foldable ? `<div class="block-children-container"><button class="block-children-left-border"></button><div class="block-children">${children}</div></div>` : ""}
    </div>`;
}

const nestedOutline = Array.from({ length: 8 }, (_, index) => index)
  .reduceRight((children, depth) => block({
    id: `depth-${depth}`,
    label: `Depth ${depth} keeps enough room for useful text.`,
    children,
  }), "");
const heading = block({
  id: "heading-parent",
  label: "Heading parent keeps its trailing touch target above its child.",
  mainClass: "bullet-h1",
  children: block({ id: "heading-leaf", label: "Heading leaf text reaches the row edge." }),
});
const embed = `
  <div class="ls-block block-embed-host" data-row="embed-host">
    <div class="block-main"><div class="block-content-wrapper"><div class="macro-host"><div class="embed-block"><div class="live-ref-group">
      ${block({
        id: "embed-live-parent",
        label: "Embedded live parent has the only disclosure control.",
        children: block({ id: "embed-live-leaf", label: "Embedded live leaf text reaches the row edge." }),
      })}
    </div></div></div></div></div>
  </div>`;
const sidebar = `<aside class="right-sidebar"><section class="rs-item"><div class="rs-item-body">${block({
  id: "sidebar-parent",
  label: "Sidebar live parent keeps the same separated control.",
  children: block({ id: "sidebar-leaf", label: "Sidebar leaf text reaches the row edge." }),
})}</div></section></aside>`;
const markup = `<main class="main-content"><div class="main-content-inner"><h1>Mobile outline</h1>${nestedOutline}${heading}${embed}${sidebar}</div></main>`;

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function attachCounters(page) {
  await page.evaluate(() => {
    window.mobileLayoutCounters = { disclosure: 0, bullet: 0, content: 0 };
    document.querySelectorAll(".collapse-toggle").forEach((element) => {
      element.addEventListener("click", () => { window.mobileLayoutCounters.disclosure += 1; });
    });
    document.querySelectorAll(".bullet-container").forEach((element) => {
      element.addEventListener("click", () => { window.mobileLayoutCounters.bullet += 1; });
    });
    document.querySelectorAll(".block-content").forEach((element) => {
      element.addEventListener("click", () => { window.mobileLayoutCounters.content += 1; });
    });
  });
}

async function counters(page) {
  return page.evaluate(() => ({ ...window.mobileLayoutCounters }));
}

async function tapCenter(page, rect) {
  await page.touchscreen.tap(rect.left + rect.width / 2, rect.top + rect.height / 2);
}

function rowTargetSelector(id, target) {
  const main = `[data-row="${id}"] > .block-main`;
  return {
    toggle: `${main} > .block-controls > .collapse-toggle`,
    bullet: `${main} > .block-controls > .bullet-container`,
    content: `${main} > .block-content-wrapper > .block-content`,
  }[target];
}

async function visibleLiveRow(page, id, target) {
  const selector = rowTargetSelector(id, target);
  await page.locator(selector).scrollIntoViewIfNeeded();
  const row = await page.evaluate(({ id: rowId, target: targetName }) => {
    const element = document.querySelector(`[data-row="${rowId}"]`);
    const main = element?.querySelector(":scope > .block-main");
    const toggle = main?.querySelector(":scope > .block-controls > .collapse-toggle");
    const bullet = main?.querySelector(":scope > .block-controls > .bullet-container");
    const content = main?.querySelector(":scope > .block-content-wrapper > .block-content");
    const wrapper = content?.parentElement;
    if (!(main && toggle && bullet && content && wrapper)) throw new Error(`mobile row ${rowId} is incomplete`);
    const rect = (node) => {
      const value = node.getBoundingClientRect();
      return { left: value.left, right: value.right, top: value.top, bottom: value.bottom, width: value.width, height: value.height };
    };
    const row = {
      id: rowId,
      foldable: toggle.classList.contains("has-children"),
      main: rect(main),
      toggle: rect(toggle),
      bullet: rect(bullet),
      content: rect(content),
      paddingRight: Number.parseFloat(getComputedStyle(wrapper).paddingRight),
    };
    const targetRect = row[targetName];
    return {
      row,
      targetRect,
      targetVisible: targetRect.width > 0 && targetRect.height > 0
        && targetRect.left >= 0 && targetRect.right <= window.innerWidth
        && targetRect.top >= 0 && targetRect.bottom <= window.innerHeight,
      viewport: { width: window.innerWidth, height: window.innerHeight },
    };
  }, { id, target });
  assert(row.targetVisible, `target is not fully visible before ${target} touch for ${id}: ${JSON.stringify(row)}`);
  return row.row;
}

async function hitTarget(page, id, target, rect) {
  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  return page.evaluate(({ selector, x, y }) => {
    const target = document.querySelector(selector);
    const element = document.elementFromPoint(x, y);
    return {
      className: element?.className || "",
      isTarget: Boolean(target && element && (element === target || target.contains(element))),
      interceptedByToggle: Boolean(element?.closest(".collapse-toggle")),
    };
  }, { selector: rowTargetSelector(id, target), x, y });
}

function assertFoldableRow(row, width) {
  const overlap = !(row.toggle.right <= row.bullet.left || row.toggle.left >= row.bullet.right || row.toggle.top >= row.bullet.bottom || row.toggle.bottom <= row.bullet.top);
  assert(Math.abs(row.toggle.right - row.main.right) <= 1, `fold target is not row-trailing for ${row.id} at ${width}px: ${JSON.stringify(row)}`);
  assert(row.toggle.width >= 44 && row.toggle.height >= 30, `fold target shrank for ${row.id} at ${width}px: ${JSON.stringify(row)}`);
  assert(!overlap, `fold target overlaps bullet for ${row.id} at ${width}px: ${JSON.stringify(row)}`);
  assert(row.content.right <= row.toggle.left + 1, `content is covered by fold target for ${row.id} at ${width}px: ${JSON.stringify(row)}`);
}

function assertLeafRow(row, width) {
  assert(row.paddingRight <= 1, `leaf reserves trailing fold padding for ${row.id} at ${width}px: ${JSON.stringify(row)}`);
  assert(Math.abs(row.content.right - row.main.right) <= 1, `leaf content does not reach its row edge for ${row.id} at ${width}px: ${JSON.stringify(row)}`);
}

async function verifyCoarsePage(browser, width) {
  const page = await browser.newPage({
    viewport: { width, height: 844 },
    isMobile: true,
    hasTouch: true,
  });
  await page.setContent(`<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1"><style>${css}</style>${markup}`);
  await attachCounters(page);
  const result = await page.evaluate(() => {
    const inner = document.querySelector(".main-content-inner");
    const root = document.querySelector('[data-row="depth-0"]');
    const deepest = document.querySelector('[data-row="depth-7"] .block-content');
    const guide = document.querySelector('[data-row="depth-0"] > .block-children-container > .block-children');
    const rootBullet = document.querySelector('[data-row="depth-0"] .bullet-container');
    if (!(inner && root && deepest && guide && rootBullet)) throw new Error("mobile nesting fixture is incomplete");
    const innerStyle = getComputedStyle(inner);
    const rootRect = root.getBoundingClientRect();
    return {
      media: matchMedia("(max-width: 640px) and (pointer: coarse)").matches,
      usable: inner.getBoundingClientRect().width - Number.parseFloat(innerStyle.paddingLeft) - Number.parseFloat(innerStyle.paddingRight),
      deepestInset: deepest.getBoundingClientRect().left - rootRect.left,
      deepestWidth: deepest.getBoundingClientRect().width,
      guideOffset: Math.abs(guide.getBoundingClientRect().left - (rootBullet.getBoundingClientRect().left + rootBullet.getBoundingClientRect().width / 2)),
      embedLiveRootActiveControlCount: document.querySelectorAll('[data-row="embed-live-parent"] > .block-main > .block-controls > .collapse-toggle.has-children').length,
      embedHostActiveControlCount: document.querySelectorAll('[data-row="embed-host"] > .block-main > .block-controls > .collapse-toggle.has-children').length,
      embedLeafPlaceholder: (() => {
        const toggle = document.querySelector('[data-row="embed-live-leaf"] > .block-main > .block-controls > .collapse-toggle');
        return toggle && !toggle.classList.contains("has-children")
          ? { count: 1, pointerEvents: getComputedStyle(toggle).pointerEvents }
          : { count: 0, pointerEvents: null };
      })(),
      rows: Array.from(document.querySelectorAll("[data-row]"), (row) => {
        const main = row.querySelector(":scope > .block-main") || row.querySelector(".block-main");
        const toggle = main?.querySelector(":scope > .block-controls > .collapse-toggle") || row.querySelector(":scope > .block-main > .block-controls > .collapse-toggle");
        const bullet = main?.querySelector(":scope > .block-controls > .bullet-container") || row.querySelector(":scope > .block-main > .block-controls > .bullet-container");
        const content = main?.querySelector(":scope > .block-content-wrapper > .block-content") || row.querySelector(":scope > .block-main > .block-content-wrapper > .block-content");
        const wrapper = content?.parentElement;
        if (!(main && toggle && bullet && content && wrapper)) return null;
        const rect = (element) => { const value = element.getBoundingClientRect(); return { left: value.left, right: value.right, top: value.top, bottom: value.bottom, width: value.width, height: value.height }; };
        return { id: row.getAttribute("data-row"), foldable: toggle.classList.contains("has-children"), main: rect(main), toggle: rect(toggle), bullet: rect(bullet), content: rect(content), paddingRight: Number.parseFloat(getComputedStyle(wrapper).paddingRight) };
      }).filter(Boolean),
    };
  });
  assert(result.media, `coarse-pointer media rule did not match at ${width}px`);
  assert(result.usable >= width - 40, `mobile content remains too narrow at ${width}px: ${JSON.stringify(result)}`);
  assert(result.deepestInset <= 165 && result.deepestWidth >= (width === 390 ? 180 : 110), `deep mobile nesting consumes too much text width at ${width}px: ${JSON.stringify(result)}`);
  assert(result.guideOffset <= 1.5, `mobile nesting guide is not aligned with its parent bullet at ${width}px: ${JSON.stringify(result)}`);
  assert(result.embedLiveRootActiveControlCount === 1, `embed live root must render exactly one active disclosure control at ${width}px: ${JSON.stringify(result)}`);
  assert(result.embedHostActiveControlCount === 0, `embed host rendered a duplicate active disclosure control at ${width}px: ${JSON.stringify(result)}`);
  assert(result.embedLeafPlaceholder.count === 1, `embed leaf must retain exactly one inert disclosure placeholder at ${width}px: ${JSON.stringify(result)}`);
  assert(result.embedLeafPlaceholder.pointerEvents === "none", `embed leaf disclosure placeholder is pointer-active at ${width}px: ${JSON.stringify(result)}`);

  for (const row of result.rows) {
    if (row.foldable) {
      assertFoldableRow(row, width);
      const liveToggleRow = await visibleLiveRow(page, row.id, "toggle");
      assertFoldableRow(liveToggleRow, width);
      const toggleHit = await hitTarget(page, row.id, "toggle", liveToggleRow.toggle);
      assert(toggleHit.isTarget, `trailing touch is intercepted before reaching disclosure for ${row.id} at ${width}px: ${JSON.stringify({ row: liveToggleRow, toggleHit })}`);
      const beforeToggle = await counters(page);
      await tapCenter(page, liveToggleRow.toggle);
      const afterToggle = await counters(page);
      assert(afterToggle.disclosure === beforeToggle.disclosure + 1 && afterToggle.bullet === beforeToggle.bullet && afterToggle.content === beforeToggle.content, `trailing touch did not reach only disclosure for ${row.id} at ${width}px: ${JSON.stringify({ beforeToggle, afterToggle, row: liveToggleRow })}`);
      const liveBulletRow = await visibleLiveRow(page, row.id, "bullet");
      assertFoldableRow(liveBulletRow, width);
      const bulletHit = await hitTarget(page, row.id, "bullet", liveBulletRow.bullet);
      assert(bulletHit.isTarget && !bulletHit.interceptedByToggle, `bullet touch is intercepted before reaching its control for ${row.id} at ${width}px: ${JSON.stringify({ row: liveBulletRow, bulletHit })}`);
      const beforeBullet = await counters(page);
      await tapCenter(page, liveBulletRow.bullet);
      const afterBullet = await counters(page);
      assert(afterBullet.bullet === beforeBullet.bullet + 1 && afterBullet.disclosure === beforeBullet.disclosure && afterBullet.content === beforeBullet.content, `bullet touch did not reach only bullet for ${row.id} at ${width}px: ${JSON.stringify({ beforeBullet, afterBullet, row: liveBulletRow })}`);
    } else {
      assertLeafRow(row, width);
      const liveContentRow = await visibleLiveRow(page, row.id, "content");
      assertLeafRow(liveContentRow, width);
      const x = Math.max(liveContentRow.content.left + 1, Math.min(liveContentRow.content.right - 4, liveContentRow.main.right - 8));
      const y = liveContentRow.content.top + liveContentRow.content.height / 2;
      assert(x >= liveContentRow.main.right - 44 && x < liveContentRow.content.right && y > liveContentRow.content.top && y < liveContentRow.content.bottom, `leaf right-edge touch is not over live content for ${row.id} at ${width}px: ${JSON.stringify({ row: liveContentRow, x, y })}`);
      const hit = await page.evaluate(({ id, x, y }) => {
        const content = document.querySelector(`[data-row="${id}"] > .block-main > .block-content-wrapper > .block-content`);
        const element = document.elementFromPoint(x, y);
        return {
          className: element?.className || "",
          intercepted: Boolean(element?.closest(".collapse-toggle")),
          isContent: Boolean(content && element && (element === content || content.contains(element))),
        };
      }, { id: row.id, x, y });
      assert(!hit.intercepted && hit.isContent, `leaf right edge is intercepted or misses content for ${row.id} at ${width}px: ${JSON.stringify({ row: liveContentRow, hit })}`);
      const beforeContent = await counters(page);
      await page.touchscreen.tap(x, y);
      const afterContent = await counters(page);
      assert(afterContent.content === beforeContent.content + 1 && afterContent.disclosure === beforeContent.disclosure && afterContent.bullet === beforeContent.bullet, `leaf right-edge touch did not reach only content for ${row.id} at ${width}px: ${JSON.stringify({ beforeContent, afterContent, row: liveContentRow, hit })}`);
    }
  }
  if (width === 390) {
    fs.mkdirSync(path.join(root, "test-results"), { recursive: true });
    await page.screenshot({ path: path.join(root, "test-results/mobile-content-width.png"), fullPage: true });
  }
  await page.close();
  return result;
}

async function verifyPrecisePage(browser) {
  const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
  await page.setContent(`<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1"><style>${css}</style>${markup}`);
  const desktop = await page.evaluate(() => {
    const root = document.querySelector('[data-row="depth-0"]');
    const child = document.querySelector('[data-row="depth-1"]');
    const fold = document.querySelector('[data-row="depth-0"] .collapse-toggle');
    if (!(root && child && fold)) throw new Error("desktop nesting fixture is incomplete");
    return { indent: child.getBoundingClientRect().left - root.getBoundingClientRect().left, foldWidth: fold.getBoundingClientRect().width, trailing: Math.abs(fold.getBoundingClientRect().right - root.querySelector(".block-main").getBoundingClientRect().right) <= 1 };
  });
  await page.close();
  assert(desktop.indent === 29 && desktop.foldWidth === 18 && !desktop.trailing, `precise-pointer outline geometry changed with the mobile fix: ${JSON.stringify(desktop)}`);
}

const browser = await chromium.launch({ headless: true });
try {
  const results = [];
  for (const width of [390, 320]) results.push(await verifyCoarsePage(browser, width));
  await verifyPrecisePage(browser);
  console.log(`PASS: coarse-pointer geometry and touchscreen hit paths hold at ${results.map((result) => Math.round(result.usable)).join("px and ")}px usable width`);
} finally {
  await browser.close();
}

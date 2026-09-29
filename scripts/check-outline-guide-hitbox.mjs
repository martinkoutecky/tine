// GH #423: browser geometry regression for the outline guide's click target.
//
// Two controls sit side by side on every parent row: the guide (folds EVERY
// descendant) and the fold arrow (folds ONE block). The reporter, three weeks
// into daily use, kept aiming at the arrow and folding the whole subtree —
// because the guide is z-index:1 and its 14px box was painted over the arrow's
// leftmost pixels. jsdom applies no CSS layout, so this is the only layer that
// can see it. The assertions are the user-visible contract, not the constants:
// the guide covers no pixel of the controls beside it, stays centred on the
// line it draws, and is no larger than the per-block control it sits next to.
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = ["theme.css", "app.css"]
  .map((file) => fs.readFileSync(path.join(root, "src/styles", file), "utf8"))
  .join("\n");

const controls = `
  <div class="block-controls">
    <span class="collapse-toggle has-children"><svg viewBox="0 0 24 24" class="triangle"><path d="M8 5l8 7-8 7z"></path></svg></span>
    <span class="bullet-container"><span class="bullet"></span></span>
  </div>`;
const leaf = (text) => `
  <div class="ls-block">
    <div class="block-main">${controls}<div class="block-content-wrapper"><div class="block-content">${text}</div></div></div>
  </div>`;
// The guide is enabled only when something below it can actually be folded, so
// the child carries its own children.
const fixture = `
  <div class="ls-block">
    <div class="block-main">${controls}<div class="block-content-wrapper"><div class="block-content">Parent</div></div></div>
    <div class="block-children-container">
      <button type="button" class="block-children-left-border" aria-label="Collapse all descendants"></button>
      <div class="block-children">
        <div class="ls-block">
          <div class="block-main">${controls}<div class="block-content-wrapper"><div class="block-content">Child</div></div></div>
          <div class="block-children-container">
            <button type="button" class="block-children-left-border" aria-label="Collapse all descendants"></button>
            <div class="block-children">${leaf("Grandchild")}</div>
          </div>
        </div>
      </div>
    </div>
  </div>`;

const browser = await chromium.launch({ headless: true, args: ["--no-sandbox", "--disable-gpu"] });
try {
  const page = await browser.newPage({ viewport: { width: 900, height: 500 } });
  await page.setContent(`<!doctype html><style>${css}</style><div class="page-blocks" style="width:700px">${fixture}</div>`);

  const geometry = await page.evaluate(() => {
    const container = document.querySelector(".block-children-container");
    const guide = container.querySelector(":scope > .block-children-left-border");
    const children = container.querySelector(":scope > .block-children");
    const childRow = children.querySelector(":scope > .ls-block > .block-main");
    const arrow = childRow.querySelector(".collapse-toggle");
    const bullet = childRow.querySelector(".bullet-container");
    const box = (el) => el.getBoundingClientRect();
    const guideBox = box(guide);
    const line = Number.parseFloat(getComputedStyle(children).borderLeftWidth);
    // The guide LINE is `.block-children`'s border-left; its centre is what the
    // hitbox must agree with, not the children box's edge.
    const lineCentre = box(children).left + line / 2;
    const owner = (x) => {
      const el = document.elementFromPoint(x, box(bullet).top + box(bullet).height / 2);
      if (!el) return "none";
      if (el.closest(".block-children-left-border")) return "guide";
      if (el.closest(".collapse-toggle")) return "arrow";
      if (el.closest(".bullet-container")) return "bullet";
      return "other";
    };
    const strip = [];
    for (let x = Math.round(guideBox.left) - 4; x <= Math.round(box(bullet).right) + 4; x++) strip.push([x, owner(x)]);
    return {
      guide: { left: guideBox.left, right: guideBox.right, width: guideBox.width, centre: guideBox.left + guideBox.width / 2 },
      arrow: { left: box(arrow).left, width: box(arrow).width },
      lineCentre,
      strip,
    };
  });

  const { guide, arrow, lineCentre, strip } = geometry;
  const detail = JSON.stringify({ guide, arrow, lineCentre });

  if (Math.abs(guide.centre - lineCentre) > 1) {
    throw new Error(`the guide's click target is not centred on the line it draws: ${detail}`);
  }
  if (guide.width > arrow.width) {
    throw new Error(`folding an entire subtree has a larger target than folding one block: ${detail}`);
  }
  // The defect itself: every pixel from the arrow's left edge rightwards must
  // answer to the arrow or the bullet, never to the fold-everything guide.
  const stolen = strip.filter(([x, who]) => x >= arrow.left && who === "guide");
  if (stolen.length > 0) {
    throw new Error(
      `the fold-all guide covers ${stolen.length}px of the per-block controls ` +
        `(x=${stolen[0][0]}..${stolen[stolen.length - 1][0]}): ${detail}`
    );
  }
  // And it stays on its own side, so the order across the row is stable.
  const seen = strip.map(([, who]) => who).filter((who, index, all) => who !== all[index - 1]);
  if (seen.indexOf("guide") > seen.indexOf("arrow")) {
    throw new Error(`the guide is on the wrong side of the fold arrow: ${seen.join(">")}`);
  }

  console.log(
    `PASS: fold-all target is ${guide.width}px centred on its line (offset ` +
      `${(guide.centre - lineCentre).toFixed(2)}px) and covers none of the ` +
      `${arrow.width}px fold-one control; row reads ${seen.join(" > ")}`
  );
} finally {
  await browser.close();
}

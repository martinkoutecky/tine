// GH #464 + GH #468: browser geometry for what a sidebar row's spare width does.
//
// The two panes want opposite answers, and both were wrong at once in v0.6.981.
//
//   LEFT sidebar (#468): the whole row navigates. A page named `test` puts a few
//   characters of text in a 222px row, and the row-shaped hover highlight
//   promises that all of it is clickable. Tine diverges from Logseq here
//   deliberately; the favourites drag is protected by rowReorder's 4px threshold,
//   not by keeping part of the row un-clickable.
//
//   RIGHT sidebar (#464): only the TITLE navigates. The markup already said so —
//   the link is an <a class="rs-item-title"> — but `flex: 1 1 auto` stretched
//   that anchor across the rest of the head, so the hand cursor followed the
//   pointer out over blank space and a press there raced the reorder drag the
//   head advertises with `cursor: grab`. That is the reporter's screenshot.
//
// jsdom applies no CSS layout, so which element sits under a given pixel is only
// answerable here. The render tests next to it assert what each element DOES.
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = ["theme.css", "app.css", "favorites.css", "rightSidebarReorder.css"]
  .map((file) => fs.readFileSync(path.join(root, "src/styles", file), "utf8"))
  .join("\n");

// A deliberately short name, which is the reported case in both issues.
const left = `
  <div class="sidebar" style="width:240px">
    <div id="sidebar-favorites-list">
      <div class="nav-page" data-row-index="0">
        <span class="nav-fav-spacer"></span>
        <span class="nav-page-label">⭐ test</span>
      </div>
    </div>
    <div id="sidebar-recent-list">
      <div class="nav-page"><span class="nav-page-label">test</span></div>
    </div>
  </div>`;

const right = `
  <div class="right-sidebar" style="width:340px">
    <div class="right-sidebar-body">
      <div class="rs-item" data-row-index="0">
        <div class="rs-item-head">
          <button class="rs-item-toggle" type="button"><span>▸</span></button>
          <a class="rs-item-title">test</a>
          <button class="rs-close" type="button">✕</button>
        </div>
      </div>
    </div>
  </div>`;

const browser = await chromium.launch({ headless: true, args: ["--no-sandbox", "--disable-gpu"] });
try {
  const page = await browser.newPage({ viewport: { width: 900, height: 500 } });
  await page.setContent(`<!doctype html><style>${css}</style>${left}${right}`);

  const measured = await page.evaluate(() => {
    const box = (el) => el.getBoundingClientRect();
    /** Walk the row's spare width and report what owns each pixel. */
    const scan = (row, label, stopAt) => {
      const rowBox = box(row);
      const labelRight = box(label).right;
      const limit = stopAt ? box(stopAt).left : rowBox.right;
      const y = rowBox.top + rowBox.height / 2;
      const owners = [];
      for (let x = Math.ceil(labelRight) + 2; x < Math.floor(limit) - 2; x++) {
        const el = document.elementFromPoint(x, y);
        owners.push(!el ? "none" : el === label || label.contains(el) ? "label" : el === row || row.contains(el) ? "row" : "outside");
      }
      return { spare: Math.round(limit - labelRight), owners, labelWidth: Math.round(box(label).width), rowWidth: Math.round(rowBox.width) };
    };

    /** How much wider the element is than the text inside it. This, not the
     *  markup, is what GH #464 was about: the anchor WAS the link already, it
     *  was just stretched across the row. */
    const overhang = (el) => {
      const range = document.createRange();
      range.selectNodeContents(el);
      return Math.round(box(el).width - range.getBoundingClientRect().width);
    };

    const fav = document.querySelector("#sidebar-favorites-list .nav-page");
    const recent = document.querySelector("#sidebar-recent-list .nav-page");
    const head = document.querySelector(".rs-item-head");
    const title = document.querySelector(".rs-item-title");
    const close = document.querySelector(".rs-close");
    return {
      favorite: scan(fav, fav.querySelector(".nav-page-label")),
      recent: scan(recent, recent.querySelector(".nav-page-label")),
      rsItem: scan(head, title, close),
      rsTitleOverhang: overhang(title),
      rsHeadSpare: Math.round(box(head).width - box(title.previousElementSibling).width - box(close).width),
      rsCloseRight: Math.round(box(close).right),
      rsHeadRight: Math.round(box(head).right),
      rsHeadPadding: Number.parseFloat(getComputedStyle(head).paddingRight),
      cursors: {
        favorite: getComputedStyle(fav).cursor,
        recent: getComputedStyle(recent).cursor,
        rsHead: getComputedStyle(head).cursor,
        rsTitle: getComputedStyle(title).cursor,
      },
    };
  });

  const fail = (message) => {
    throw new Error(`${message}\n${JSON.stringify(measured, null, 2)}`);
  };

  // A row with no spare width proves nothing about who owns the spare width, so
  // the fixture has to actually be the reported shape first. The right sidebar's
  // spare width is measured from the head, not from the title: a stretched title
  // is the defect, and it must not be able to make this precondition vacuous.
  for (const key of ["favorite", "recent"]) {
    if (measured[key].spare < 40) fail(`${key}: fixture is not the reported shape — only ${measured[key].spare}px of spare width`);
  }
  if (measured.rsHeadSpare < 40) fail(`right sidebar: fixture is not the reported shape — only ${measured.rsHeadSpare}px beside the title`);

  // GH #464, the defect itself: the anchor must be sized to its own text.
  if (measured.rsTitleOverhang > 2) {
    fail(`right sidebar: the title anchor is ${measured.rsTitleOverhang}px wider than the text in it — that overhang is the reported hitbox`);
  }

  // GH #468 — the left sidebar's spare width belongs to the row.
  for (const key of ["favorite", "recent"]) {
    const stray = measured[key].owners.filter((who) => who !== "row" && who !== "label");
    if (stray.length > 0) fail(`${key}: ${stray.length}px of the row's spare width is not the row (${[...new Set(stray)].join(",")})`);
    if (measured.cursors[key] !== "pointer") fail(`${key}: the row does not advertise itself as clickable (cursor: ${measured.cursors[key]})`);
  }

  // GH #464 — the right sidebar's spare width does NOT belong to the link.
  const claimed = measured.rsItem.owners.filter((who) => who === "label");
  if (claimed.length > 0) {
    fail(`right sidebar: the title anchor still covers ${claimed.length}px right of its own text — this is the reported hitbox`);
  }
  // The head advertises the reorder drag (og gained right-sidebar reorder in
  // 70c5e7bad; the old relaxed "not pointer" assertion was stale).
  if (measured.cursors.rsHead !== "grab") {
    fail(`right sidebar: the head no longer advertises the reorder drag (cursor: ${measured.cursors.rsHead})`);
  }
  // The close button must not have followed the title leftwards.
  if (measured.rsHeadRight - measured.rsCloseRight > measured.rsHeadPadding + 2) {
    fail(`right sidebar: the close button left the right edge (${measured.rsHeadRight - measured.rsCloseRight}px in from the head edge)`);
  }

  console.log(
    `PASS: left rows keep the whole width clickable (favourites ${measured.favorite.labelWidth}px name ` +
      `in a ${measured.favorite.rowWidth}px row, ${measured.favorite.spare}px spare, all of it the row); ` +
      `right-sidebar title is ${measured.rsItem.labelWidth}px and claims none of its ${measured.rsItem.spare}px ` +
      `of spare width, which the head owns; close button still at the right edge`
  );
} finally {
  await browser.close();
}

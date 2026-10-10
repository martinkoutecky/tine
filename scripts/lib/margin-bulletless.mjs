// Slice 2b (margin dialogue): comment blocks draw no bullets; replies nest by a
// thin rule plus a small capped indent. Shared by the margin shot scripts:
// `bulletlessFacts` runs inside the page, `checkBulletless` judges the result.

export function bulletlessFacts() {
  const cards = [...document.querySelectorAll(".ls-block.comment-card")];
  const levels = {};
  for (const card of cards) {
    for (const children of card.querySelectorAll(".block-children")) {
      let level = 0;
      for (let el = children; el && el !== card; el = el.parentElement) if (el.classList.contains("block-children")) level++;
      const style = getComputedStyle(children);
      (levels[level] ??= []).push({ padding: style.paddingLeft, rule: style.borderLeftWidth, margin: style.marginLeft });
    }
  }
  const plainRows = [...document.querySelectorAll(".page-blocks .ls-block")].filter((el) => !el.classList.contains("comment-in-thread"));
  // Distance from a depth-0 comment's left edge to its text.
  const textInset = cards.map((card) => {
    const text = card.querySelector(":scope > .block-main .block-content");
    return text ? Math.round((text.getBoundingClientRect().left - card.getBoundingClientRect().left) * 10) / 10 : null;
  });
  return {
    cards: cards.length,
    bulletsInCards: cards.reduce((n, c) => n + c.querySelectorAll(".bullet-container, .collapse-toggle, .block-controls").length, 0),
    handles: cards.reduce((n, c) => n + c.querySelectorAll(".comment-handle").length, 0),
    commentRows: cards.reduce((n, c) => n + 1 + c.querySelectorAll(".ls-block").length, 0),
    plainRows: plainRows.length,
    plainRowsWithBullet: plainRows.filter((el) => el.querySelector(":scope > .block-main > .block-controls > .bullet-container")).length,
    textInset,
    levels,
  };
}

export function checkBulletless(name, f, { expectCards }) {
  const fail = (why) => { throw new Error(`${name}: ${why}\n${JSON.stringify(f)}`); };
  if (f.cards !== expectCards) fail(`${f.cards} comment cards, expected ${expectCards}`);
  if (f.bulletsInCards !== 0) fail(`${f.bulletsInCards} bullets/fold arrows inside comment cards`);
  if (f.handles !== f.commentRows) fail(`${f.handles} chip handles for ${f.commentRows} comment rows`);
  if (f.plainRowsWithBullet !== f.plainRows) fail(`${f.plainRows - f.plainRowsWithBullet} ordinary rows lost their bullet`);
  for (const inset of f.textInset) if (inset === null || inset < 7 || inset > 10) fail(`comment text starts ${inset}px from the card edge, expected its 8px padding`);
  for (const [level, rows] of Object.entries(f.levels)) {
    for (const row of rows) {
      const want = Number(level) <= 2 ? "10px" : "0px";
      if (row.padding !== want) fail(`reply nesting level ${level} indent ${row.padding}, expected ${want}`);
      if (row.rule !== "2px") fail(`reply nesting level ${level} has no rule (${row.rule})`);
      if (row.margin !== "0px") fail(`reply nesting level ${level} outline margin ${row.margin}`);
    }
  }
}

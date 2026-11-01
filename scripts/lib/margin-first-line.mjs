// Runs unchanged in Chromium and native WebKit. Text ranges, rather than the
// body's full-width box, check that response controls do not cover its text.
export function marginFirstLineProblems() {
  const problems = [];
  const intersects = (a, b) => a.left < b.right - 0.5 && a.right > b.left + 0.5 && a.top < b.bottom - 0.5 && a.bottom > b.top + 0.5;
  for (const card of document.querySelectorAll(".comment-in-thread")) {
    const del = card.querySelector(":scope > .comment-header .comment-delete");
    if (!del) continue;
    const d = del.getBoundingClientRect();
    if (!d.width || !d.height) continue;
    const id = card.dataset.blockId;
    const quote = card.querySelector(":scope > .comment-quote .comment-quote-text");
    const body = card.querySelector(":scope > .block-main .block-content");
    const chip = card.querySelector(":scope > .comment-header .author-chip");
    const texts = [];
    for (const content of [quote, body].filter(Boolean)) {
      const bounds = content.getBoundingClientRect();
      const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
      while (walker.nextNode()) {
        const node = walker.currentNode;
        if (!node.textContent.trim()) continue;
        const range = document.createRange();
        range.selectNodeContents(node);
        for (const r of range.getClientRects()) {
          const visible = { left: Math.max(r.left, bounds.left), right: Math.min(r.right, bounds.right), top: r.top, bottom: r.bottom };
          if (visible.right > visible.left && visible.bottom > visible.top) texts.push({ ...visible, quote: content === quote });
        }
      }
    }
    if (d.width < 24 || d.height < 24) problems.push(`${id}: delete target below 24px`);
    if (chip && intersects(d, chip.getBoundingClientRect())) problems.push(`${id}: delete overlaps author`);
    if (texts.some(r => intersects(d, r))) problems.push(`${id}: delete overlaps quote/body text`);
    const c = chip?.getBoundingClientRect();
    if (!c || c.bottom <= d.top || c.top >= d.bottom) problems.push(`${id}: author and delete need the same header row`);
  }
  return problems;
}

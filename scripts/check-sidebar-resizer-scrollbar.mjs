// GH #435: browser geometry regression for the left sidebar's resize strip.
//
// The strip is absolutely positioned over the sidebar's right edge — the same
// column a classic (Windows) scrollbar occupies. Two Windows users reported the
// consequence: the sidebar resizes fine, but the scrollbar underneath it can
// never be grabbed. jsdom applies no CSS layout, so this is the only layer that
// can see it. The contract asserted is the user outcome — the scrollbar and the
// resize strip occupy disjoint columns, so moving a few pixels left off the
// strip reaches the bar — not the particular widths.
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = ["theme.css", "app.css"]
  .map((file) => fs.readFileSync(path.join(root, "src/styles", file), "utf8"))
  .join("\n");

const rows = Array.from({ length: 80 }, (_, i) => `<div style="height:26px">Row ${i}</div>`).join("");
const shell = `
  <div class="app-container">
    <div class="left-sidebar" style="flex:0 0 240px;width:240px">
      <div class="left-sidebar-scroll">${rows}</div>
      <div class="sidebar-resizer"></div>
    </div>
  </div>`;

// This Chromium always draws overlay scrollbars, which consume no layout width
// and therefore cannot reproduce the reported condition at all. The reporters
// are on Windows, where WebView2 draws a classic bar INSIDE the layout — that
// bar is what the resize strip was sitting on. `scrollbar-gutter: stable`
// reserves exactly that column here (`::-webkit-scrollbar` widths are ignored
// by this build). Fixture-only: production sets no scrollbar width or gutter,
// so overlay platforms keep their overlay bars.
const windowsClassicScrollbar = `.left-sidebar-scroll { scrollbar-gutter: stable; }`;

const browser = await chromium.launch({ headless: true, args: ["--no-sandbox", "--disable-gpu"] });
try {
  const page = await browser.newPage({ viewport: { width: 900, height: 400 } });
  await page.setContent(`<!doctype html><style>${css}${windowsClassicScrollbar}</style>${shell}`);

  const geometry = await page.evaluate(() => {
    const scroll = document.querySelector(".left-sidebar-scroll");
    const resizer = document.querySelector(".sidebar-resizer");
    const style = getComputedStyle(scroll);
    const borderRight = Number.parseFloat(style.borderRightWidth);
    const borderLeft = Number.parseFloat(style.borderLeftWidth);
    // A classic scrollbar consumes layout width; an overlay one does not, and
    // then this browser cannot answer the question at all.
    const scrollbarWidth = scroll.offsetWidth - scroll.clientWidth - borderLeft - borderRight;
    const paddingBoxRight = scroll.getBoundingClientRect().right - borderRight;
    return {
      scrollbar: { left: paddingBoxRight - scrollbarWidth, right: paddingBoxRight, width: scrollbarWidth },
      resizer: (({ left, right, width }) => ({ left, right, width }))(resizer.getBoundingClientRect()),
      overflows: scroll.scrollHeight > scroll.clientHeight,
      gutter: borderRight,
    };
  });

  const detail = JSON.stringify(geometry);
  if (!geometry.overflows) throw new Error(`fixture does not scroll, so it proves nothing: ${detail}`);
  if (geometry.scrollbar.width <= 0) {
    // The fixture asked for a classic bar; without one nothing was measured, so
    // fail rather than pass on an assertion that was never evaluated.
    throw new Error(`the fixture produced no layout-consuming scrollbar to test against: ${detail}`);
  }
  if (geometry.scrollbar.right > geometry.resizer.left) {
    throw new Error(
      `the resize strip covers ${(geometry.scrollbar.right - geometry.resizer.left).toFixed(1)}px of the ` +
        `sidebar scrollbar, so the bar cannot be grabbed: ${detail}`
    );
  }

  console.log(
    `PASS: sidebar scrollbar occupies ${geometry.scrollbar.width}px ending at ` +
      `${geometry.scrollbar.right}px and the ${geometry.resizer.width}px resize strip starts at ` +
      `${geometry.resizer.left}px — disjoint columns`
  );
} finally {
  await browser.close();
}

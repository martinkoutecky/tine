import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = ["theme.css", "app.css"]
  .map((file) => fs.readFileSync(path.join(root, "src/styles", file), "utf8"))
  .join("\n");
const actions = (extraClass = "") => `
  <div class="editor-wrap">
    <div class="sel-toolbar ${extraClass}">
      <button data-selection-action="bold">B</button><button data-selection-action="italic">I</button><button class="sel-action-page-link" data-selection-action="page-link">[[ ]]</button><button data-selection-action="code">\`</button>
      <div class="sel-toolbar-secondary"><button data-selection-action="link">Link</button><button data-selection-action="strike">S</button><button data-selection-action="highlight">H</button></div>
      <button class="sel-toolbar-more">…</button>
    </div>
  </div>`;

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 600, height: 240 } });
  await page.setContent(`<!doctype html><style>${css}</style><div id="narrow" style="width:220px">${actions()}</div><div id="wide" style="width:400px;margin-top:80px">${actions()}</div>`);
  const geometry = await page.evaluate(() => {
    const read = (id) => {
      const host = document.querySelector(`#${id} .editor-wrap`);
      const toolbar = host.querySelector(".sel-toolbar");
      return {
        hostWidth: host.getBoundingClientRect().width,
        toolbarWidth: toolbar.getBoundingClientRect().width,
        secondary: getComputedStyle(host.querySelector(".sel-toolbar-secondary")).display,
        more: getComputedStyle(host.querySelector(".sel-toolbar-more")).display,
      };
    };
    return { narrow: read("narrow"), wide: read("wide") };
  });
  if (geometry.narrow.secondary !== "none" || geometry.narrow.more === "none" || geometry.narrow.toolbarWidth > geometry.narrow.hostWidth) {
    throw new Error(`narrow selection toolbar clips instead of using overflow: ${JSON.stringify(geometry)}`);
  }
  if (geometry.wide.secondary === "none" || geometry.wide.more !== "none") {
    throw new Error(`wide selection toolbar needlessly hides actions: ${JSON.stringify(geometry)}`);
  }
  console.log(`PASS: desktop selection toolbar is ${geometry.narrow.toolbarWidth}px narrow and expands at ${geometry.wide.hostWidth}px`);

  // GH #375: Android's native ActionMode occupies the selection-adjacent band.
  // Tine's own mobile formatting surface must instead form a second dock row
  // immediately above the existing keyboard toolbar. The custom property is
  // the real runtime handoff published by MobileKeyboardToolbar.tsx.
  await page.setViewportSize({ width: 320, height: 844 });
  await page.setContent(`<!doctype html><style>${css}</style>
    <div id="mobile">
      <div id="native-action-mode-proxy" style="position:fixed;left:20px;right:20px;top:120px;height:140px"></div>
      <div style="position:absolute;left:18px;top:220px;width:220px">
        ${actions("sel-toolbar-mobile")}
      </div>
      <div class="mobile-keyboard-toolbar" style="top:700px">
        <div class="mobile-keyboard-toolbar-strip"><button class="mobile-keyboard-toolbar-btn">A</button></div>
      </div>
    </div>`);
  await page.evaluate(() => {
    const keyboardTop = document.querySelector(".mobile-keyboard-toolbar").getBoundingClientRect().top;
    document.documentElement.style.setProperty("--mobile-kb-toolbar-lift", `${window.innerHeight - keyboardTop + 8}px`);
  });
  const mobile = await page.evaluate(() => {
    const rect = (selector) => {
      const r = document.querySelector(selector).getBoundingClientRect();
      return { top: r.top, right: r.right, bottom: r.bottom, left: r.left, width: r.width, height: r.height };
    };
    return {
      formatting: rect(".sel-toolbar-mobile"),
      keyboard: rect(".mobile-keyboard-toolbar"),
      nativeProxy: rect("#native-action-mode-proxy"),
      position: getComputedStyle(document.querySelector(".sel-toolbar-mobile")).position,
      visibleActions: [...document.querySelectorAll(".sel-toolbar-mobile [data-selection-action]")]
        .filter((button) => {
          const style = getComputedStyle(button);
          const box = button.getBoundingClientRect();
          return style.display !== "none" && style.visibility !== "hidden" && box.width > 0 && box.height > 0;
        })
        .map((button) => button.getAttribute("data-selection-action")),
      more: getComputedStyle(document.querySelector(".sel-toolbar-mobile > .sel-toolbar-more")).display,
    };
  });
  const overlaps = (a, b) => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
  if (mobile.position !== "fixed") {
    throw new Error(`mobile formatting toolbar remains editor-anchored instead of docked: ${JSON.stringify(mobile)}`);
  }
  if (overlaps(mobile.formatting, mobile.nativeProxy)) {
    throw new Error(`mobile formatting toolbar still occupies native ActionMode's selection band: ${JSON.stringify(mobile)}`);
  }
  const gap = mobile.keyboard.top - mobile.formatting.bottom;
  if (gap < 4 || gap > 12) {
    throw new Error(`mobile formatting toolbar is not immediately above the keyboard dock (gap ${gap}px): ${JSON.stringify(mobile)}`);
  }
  const expectedActions = ["bold", "italic", "page-link", "code", "link", "strike", "highlight"];
  if (JSON.stringify(mobile.visibleActions) !== JSON.stringify(expectedActions) || mobile.more !== "none") {
    throw new Error(`mobile formatting actions remain collapsed behind More: ${JSON.stringify(mobile)}`);
  }
  if (mobile.formatting.left < 6 || mobile.formatting.right > 314) {
    throw new Error(`mobile formatting dock escapes phone safe horizontal bounds: ${JSON.stringify(mobile)}`);
  }
  await page.screenshot({ path: "/tmp/selection-toolbar-mobile.png" });
  console.log(`PASS: mobile formatting dock avoids ActionMode and sits ${gap}px above the keyboard toolbar`);
} finally {
  await browser.close();
}

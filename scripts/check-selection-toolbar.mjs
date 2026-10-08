import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const css = ["theme.css", "app.css"]
  .map((file) => fs.readFileSync(path.join(root, "src/styles", file), "utf8"))
  .join("\n");
const actions = `
  <div class="editor-wrap">
    <div class="sel-toolbar">
      <button>B</button><button>I</button><button class="sel-action-page-link">[[ ]]</button><button>\`</button>
      <div class="sel-toolbar-secondary"><button>Link</button><button>S</button><button>H</button></div>
      <button class="sel-toolbar-more">…</button>
    </div>
  </div>`;

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 600, height: 240 } });
  await page.setContent(`<!doctype html><style>${css}</style><div id="narrow" style="width:220px">${actions}</div><div id="wide" style="width:400px;margin-top:80px">${actions}</div>`);
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
  console.log(`PASS: selection toolbar is ${geometry.narrow.toolbarWidth}px narrow and expands at ${geometry.wide.hostWidth}px`);
} finally {
  await browser.close();
}

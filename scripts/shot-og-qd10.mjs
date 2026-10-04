// GH #510: actual Block click, code-body editing and boundary return in Chromium.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { writeFileSync, rmSync } from "node:fs";
import assert from "node:assert/strict";
const html = ".qd10-shot.html", entry = ".qd10-shot.tsx", port = 5230;
writeFileSync(html, `<html data-theme="light"><body><div id="root"></div><script type="module" src="/${entry}"></script></body></html>`);
writeFileSync(entry, `
import { render } from "solid-js/web";
import "./src/styles/inter.css";
import "./src/styles/theme.css";
import "./src/styles/editableEmoji.css";
import "./src/styles/app.css";
import { initParser } from "./src/render/parse";
import { node, resetStore, undo } from "./src/document";
import { loadSingle } from "./src/document/workingSet";
import { Block } from "./src/components/Block";
import { endEdit } from "./src/editorController";
await initParser();
window.raw = () => node("mixed").raw;
window.undoEdit = undo;
let dispose;
window.fixture = (raw, format) => { dispose?.(); endEdit("page-navigation"); resetStore(); loadSingle({ name: "QD10", title: "QD10", kind: "page", format, pre_block: null,
blocks: [{ id: "mixed", raw, collapsed: false, children: [] }] });
dispose = render(() => <main style="width:620px;margin:40px auto"><Block id="mixed"/></main>, document.getElementById("root")); };
`);
const server = spawn("./node_modules/.bin/vite", ["--host", "127.0.0.1", "--port", String(port), "--strictPort"], { stdio: "inherit", env: { ...process.env, CHOKIDAR_USEPOLLING: "1", CHOKIDAR_INTERVAL: "1000" } });
let browser;
try {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${port}/${html}`)).ok) break; } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const tab = await browser.newPage({ viewport: { width: 960, height: 680 } });
  const errors = []; tab.on("pageerror", error => errors.push(String(error)));
  tab.setDefaultTimeout(15000);
  await tab.goto(`http://127.0.0.1:${port}/${html}`);
  await tab.waitForFunction(() => !!window.fixture);
  for (const theme of ["light", "dark"]) for (const format of ["md", "org"]) {
    const open = format === "md" ? "```js" : "#+BEGIN_SRC js";
    const close = format === "md" ? "```" : "#+END_SRC";
    const raw = `Prose before\n${open}\nfirst\n${close}\nMiddle prose\n${open}\nsecond\n${close}\nProse after`;
    await tab.evaluate(([raw, format, theme]) => { document.documentElement.dataset.theme = theme; window.fixture(raw, format); }, [raw, format, theme]);
    await tab.locator("pre.code-block").nth(1).click({ position: { x: 30, y: 18 } });
    const editor = tab.locator("textarea.code-edit");
    assert.equal(await editor.inputValue(), "second");
    await tab.screenshot({ path: `/tmp/og-qd10-${theme}-${format}-body.png` });
    await editor.fill("changed\n- literal");
    assert.equal(await tab.evaluate(() => window.raw()), raw.replace("second", "changed\n- literal"));
    await editor.press("Home"); await editor.press("Control+Home"); await editor.press("Backspace");
    assert.equal(await tab.locator("textarea").inputValue(), raw.replace("second", "changed\n- literal"));
    await tab.screenshot({ path: `/tmp/og-qd10-${theme}-${format}-raw.png` });
    await tab.evaluate(() => window.undoEdit());
    assert.equal(await tab.evaluate(() => window.raw()), raw);
  }
  assert.deepEqual(errors, []);
  console.log("GH #510: Chromium mixed Markdown/Org second-fence click, edit, raw boundary and undo passed in both themes");
} finally {
  await browser?.close(); server.kill(); rmSync(html, { force: true }); rmSync(entry, { force: true });
}

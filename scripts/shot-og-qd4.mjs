// Focused Chromium proof of the real slash picker and its complete draft commit.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { writeFileSync, rmSync } from "node:fs";
import assert from "node:assert/strict";
const html = ".qd4-shot.html", entry = ".qd4-shot.tsx", port = 5234;
writeFileSync(html, `<html data-theme="light"><body><div id="root"></div><script type="module" src="/${entry}"></script></body></html>`);
writeFileSync(entry, `
import { render } from "solid-js/web";
import "./src/styles/theme.css";
import "./src/styles/app.css";
import { initParser } from "./src/render/parse";
import { loadSingle } from "./src/document/workingSet";
import { Block } from "./src/components/Block";
import { DatePicker } from "./src/components/DatePicker";
import { startEditing } from "./src/editorController";
import { node } from "./src/document";
await initParser();
loadSingle({ name: "QD4", kind: "page", title: "QD4", format: "md", pre_block: null,
  blocks: [{ id: "task", raw: "TODO Plan meeting", collapsed: false, children: [] }] });
render(() => <main style="max-width:700px;margin:160px auto"><h1>Date picker</h1>
  <Block id="task" /><DatePicker /></main>, document.getElementById("root"));
startEditing("task", 17);
window.qd4Raw = () => node("task").raw;
`);
const server = spawn("./node_modules/.bin/vite", ["--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://localhost:${port}/${html}`)).ok) break; } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const tab = await browser.newPage({ viewport: { width: 390, height: 600 } });
  await tab.goto(`http://localhost:${port}/${html}`);
  const editor = tab.locator("textarea.block-editor");
  await editor.fill("TODO Plan meeting /scheduled");
  await tab.locator(".ac-item").filter({ hasText: /^Scheduled$/ }).click();
  await tab.locator('[data-day="12"]').click();
  await tab.locator(".dp-addtime").click();
  await tab.locator(".dp-time-input").fill("10:00");
  await tab.locator(".dp-rep-unit").selectOption("w");
  assert.ok(!(await tab.evaluate(() => window.qd4Raw())).includes("SCHEDULED:"), "day selection remains a draft");
  for (const theme of ["light", "dark"]) {
    await tab.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    const box = await tab.locator(".date-picker").boundingBox();
    assert.ok(box.y >= 0 && box.y + box.height <= 600, "complete picker stays in the viewport");
    await tab.screenshot({ path: `/tmp/og-qd4-${theme}.png` });
  }
  await tab.getByRole("button", { name: "Done", exact: true }).click();
  assert.match(await tab.evaluate(() => window.qd4Raw()), /SCHEDULED: <\d{4}-\d{2}-12 \w+ 10:00 \+1w>/);
  console.log("Chromium: day-first time/repeat draft + Done; viewport and light/dark screenshots passed");
} finally {
  await browser?.close(); server.kill(); rmSync(html, { force: true }); rmSync(entry, { force: true });
}

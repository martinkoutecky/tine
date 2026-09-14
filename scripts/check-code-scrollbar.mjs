// Run under xvfb-run on Linux: this probes an actual classic scrollbar.
import { chromium } from "playwright";
import { createServer } from "vite";
import { realpathSync } from "node:fs";
import assert from "node:assert/strict";
const server = await createServer({server:{host:"127.0.0.1",port:0,fs:{allow:[process.cwd(),realpathSync("node_modules/inter-ui"),realpathSync("node_modules/@fontsource-variable/noto-emoji")]}}});
await server.listen();
const browser = await chromium.launch({headless:false,args:["--no-sandbox","--disable-features=OverlayScrollbar"]});
try {
 const page = await browser.newPage();
 for (const zoom of [1,1.25]) {
  await page.goto(`${server.resolvedUrls.local[0]}scripts/fixtures/drag-selection/index.html?shape=code&zoom=${zoom}`);
  const code = page.locator(".code-block"); await code.waitFor(); await page.evaluate(()=>document.fonts.ready);
  const box = await code.evaluate(e=>({rect:e.getBoundingClientRect().toJSON(),gutter:e.offsetHeight-e.clientHeight}));
  assert.ok(box.gutter>0,"test requires an actual classic scrollbar");
  await page.mouse.move(box.rect.x+40*zoom,box.rect.bottom-6*zoom);
  await page.mouse.down(); await page.mouse.move(box.rect.x+180*zoom,box.rect.bottom-6*zoom,{steps:8}); await page.mouse.up();
  assert.equal(await page.locator("textarea.block-editor").count(),0,"scrollbar drag must not enter editing");
  assert.ok(await code.evaluate(e=>e.scrollLeft)>0,"native scrollbar must scroll the rendered code");
  await page.mouse.click(box.rect.x+40*zoom,box.rect.y+20*zoom);
  assert.equal(await page.locator("textarea.block-editor").count(),1,"code text must still enter editing");
  console.log(JSON.stringify({zoom,scrollbarDrag:true,textEdit:true}));
 }
} finally {await browser.close();await server.close();}

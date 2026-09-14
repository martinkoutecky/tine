import { createServer } from "vite";
import { chromium } from "playwright";
import { mkdir, writeFile } from "node:fs/promises";
import { realpathSync } from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = process.env.TINE_PROBE_OUT ?? path.join(root, "screenshots/pdf-toolbar");
await mkdir(out, {recursive:true});
const baseline = process.env.TINE_PDF_BASELINE_REF;
const plugins = baseline ? [{name:"before-css", enforce:"pre", load(id) {
  return id.split("?")[0] === path.join(root,"src/styles/app.css")
    ? execFileSync("git",["show",`${baseline}:src/styles/app.css`],{cwd:root,encoding:"utf8"}) : null;
}}] : [];
const server = await createServer({root,plugins,server:{host:"127.0.0.1",port:0,
  fs:{allow:[root,realpathSync(path.join(root,"node_modules/inter-ui"))]}}});
await server.listen();
const browser = await chromium.launch({args:["--no-sandbox"]});
const results=[], failures=[];
try {
 for (const [width, zoom] of [[280,1],[320,1],[320,1.1],[360,1],[1100,1]]) {
  const page=await browser.newPage({viewport:{width,height:640},hasTouch:true});
  page.on("pageerror", e => console.error(e.message));
  await page.goto(`${server.resolvedUrls.local[0]}scripts/fixtures/pdf-toolbar/index.html`);
  await page.waitForSelector('[data-pdf-ready="true"]');
  await page.evaluate(zoom => { document.documentElement.style.zoom=String(zoom); return document.fonts.ready; }, zoom);
  const geometry=await page.evaluate(() => [...document.querySelectorAll('.pdf-toolbar button,.pdf-page-input')]
   .filter(el=>getComputedStyle(el).display!=="none").map(el=>{
    const r=el.getBoundingClientRect(), x=r.x+r.width/2,y=r.y+r.height/2;
    const hit=document.elementFromPoint(x,y);
    return {label:el.getAttribute("title"),rect:r.toJSON(),hittable:r.width>0&&r.height>0&&!!hit&&(el===hit||el.contains(hit))};
   }));
  const missing=geometry.filter(x=>!x.hittable).map(x=>x.label);
  results.push({width,zoom,geometry});
  await page.screenshot({path:path.join(out,`${width}-${zoom}.png`)});
  if(missing.length){failures.push(`${width}@${zoom}: ${missing.join(", ")}`);await page.close();continue;}
  await page.locator('[title^="Find in document"]').tap();
  const findBounds = await page.locator('.pdf-find-bar').boundingBox();
  const toolbarBounds = await page.locator('.pdf-toolbar').boundingBox();
  if (findBounds.y < toolbarBounds.y+toolbarBounds.height-1) failures.push(`${width}@${zoom}: Find overlaps toolbar`);
  await page.locator('.pdf-find-bar button[title="Close (Esc)"]').tap();
  await page.locator('[aria-label="More settings"]').tap();
  await page.waitForSelector('[aria-label="PDF settings"]');
  const settingsBounds=await page.locator('[aria-label="PDF settings"]').boundingBox();
  if(settingsBounds.y < toolbarBounds.y+toolbarBounds.height-1) failures.push(`${width}@${zoom}: settings overlaps toolbar`);
  if(width<=520) {
   await page.locator('.pdf-settings-overflow button').filter({hasText:/^Notes$/}).tap();
   if(await page.evaluate(()=>window.pdfProbe.notes)!==1) failures.push(`${width}@${zoom}: Notes did not activate`);
  } else await page.locator('[aria-label="More settings"]').tap();
  // Native button keyboard activation stays available in the same DOM.
  await page.locator('[aria-label="Close PDF"]').focus();await page.keyboard.press("Enter");
  if(await page.evaluate(()=>window.pdfProbe.close)!==1) failures.push(`${width}@${zoom}: keyboard Close did not activate`);
  await page.locator('[aria-label="Close PDF"]').tap();
  if(await page.evaluate(()=>window.pdfProbe.close)!==2) failures.push(`${width}@${zoom}: touch Close did not activate`);
  await page.close();
 }
 await writeFile(path.join(out,"results.json"),JSON.stringify({results,failures},null,2));
 console.log(JSON.stringify({failures}));if(failures.length)process.exitCode=1;
}finally{await browser.close();await server.close();}

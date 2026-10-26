// GH #643/#641/#510: native scroll/toggle, select an embed host, mixed code.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = path.resolve(process.env.E2E_ARTIFACT_DIR || "artifacts/editor-2");
fs.mkdirSync(OUT, { recursive: true });
const RUN = fs.mkdtempSync(path.join(OUT, "native-")), GRAPH = path.join(RUN, "graph");
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), {recursive:true});
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(RUN, dir));
const sourceId = "e2100000-0000-4000-8000-000000000510";
fs.writeFileSync(path.join(GRAPH,"logseq/config.edn"),"{}\n");
const source = `- AAA\n  \`\`\`js\n  BBB\n  CCC\n  \`\`\`\n  DDD\n  id:: ${sourceId}\n`;
fs.writeFileSync(path.join(GRAPH,"pages/Code source.md"), source);
fs.writeFileSync(path.join(GRAPH,"pages/Editor context.md"), `- Above embed\n- {{embed ((${sourceId}))}}\n- ((${sourceId}))\n- Root **ancestor**\n  - Branch\n${Array.from({length:180},(_,i)=>`    - Child ${i}`).join("\n")}\n`);
const now = new Date(), day = `${now.getFullYear()}_${String(now.getMonth()+1).padStart(2,"0")}_${String(now.getDate()).padStart(2,"0")}`;
fs.writeFileSync(path.join(GRAPH,`journals/${day}.md`), "- [[Editor context]]\n");
const APP = process.env.TINE_APP || path.join(ROOT,"target/release/tine");
const TD = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const port = Number(process.env.E2E_DRIVER_PORT || 4746);
const env = {...process.env,TINE_GRAPH:GRAPH,XDG_DATA_HOME:path.join(RUN,"data"),XDG_CONFIG_HOME:path.join(RUN,"config"),XDG_CACHE_HOME:path.join(RUN,"cache"),WEBKIT_DISABLE_DMABUF_RENDERER:"1",WEBKIT_DISABLE_COMPOSITING_MODE:"1",LIBGL_ALWAYS_SOFTWARE:"1",GDK_BACKEND:"x11"};
const log = fs.openSync(path.join(RUN,"driver.log"),"w");
const driver = spawn(TD,["--port",String(port),"--native-port",String(port+1),"--native-driver",process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],{env,stdio:["ignore",log,log],detached:true});
let browser, stage = "startup";
const tested = "88700137bc981033ba78a8896424464b6c1d2fdc + editor-2 working diff";
try {
  browser = await remote({hostname:"127.0.0.1",port,path:"/",logLevel:"error",connectionRetryCount:2,connectionRetryTimeout:30000,capabilities:{browserName:"wry","wdio:enforceWebDriverClassic":true,"tauri:options":{application:APP}}});
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({timeout:30000});
  await openPageByName(browser,"Editor context");
  stage = "mixed code and reference";
  await browser.$(".embed-block pre.code-block code").waitForExist({timeout:15000});
  await browser.$(".block-ref pre.code-block code").waitForExist({timeout:15000});
  const measure = () => browser.execute(() => {
    const el = document.querySelector(".embed-block pre.code-block, .embed-block textarea.code-edit");
    const r=el.getBoundingClientRect(), s=getComputedStyle(el);
    return {left:r.left+parseFloat(s.paddingLeft)+parseFloat(s.borderLeftWidth),top:r.top,height:r.height,text:el.closest(".ls-block").textContent};
  });
  const before = await measure();
  await browser.$(".embed-block pre.code-block code").click();
  await browser.$(".embed-block textarea.code-edit").waitForExist({timeout:10000});
  const after = await measure();
  assert.ok(after.text.includes("AAA") && after.text.includes("DDD"),"surrounding text stays visible");
  assert.ok(Math.abs(after.left-before.left)<2 && Math.abs(after.top-before.top)<2,"mixed code stays aligned");
  await browser.saveScreenshot(path.join(RUN,"mixed-edit.png"));
  stage = "palette selects embedding host";
  await browser.keys(["Control","Shift","p","NULL"]);
  const input = await browser.$(".switcher-input");
  await input.waitForDisplayed({timeout:10000});
  await input.setValue("Select embedding block");
  await browser.keys(["Enter"]);
  await browser.waitUntil(()=>browser.execute(()=>!!document.querySelector(".block-embed-host > .block-main.selected") && !document.querySelector("textarea.block-editor")),{timeout:10000});
  assert.equal(fs.readFileSync(path.join(GRAPH,"pages/Code source.md"),"utf8"),source,"selecting the host does not mutate the source");
  // Start the scroll journey after a normal navigation boundary, with the
  // completed editor/palette transaction no longer owning a scroll anchor.
  await openPageByName(browser,"Code source");
  await openPageByName(browser,"Editor context");
  stage = "sticky scroll and click";
  await browser.executeAsync(done=>requestAnimationFrame(()=>requestAnimationFrame(()=>done())));
  const scrolled = await browser.execute(() => {
    const scroller=document.querySelector(".main-content");
    scroller.scrollTop=1200;
    return {scroll:scroller.scrollTop,height:scroller.clientHeight,total:scroller.scrollHeight};
  });
  fs.writeFileSync(path.join(RUN,"scroll-start.json"),JSON.stringify(scrolled));
  await browser.waitUntil(()=>browser.execute(()=> {
    const text=[...document.querySelectorAll('.sticky-ancestor button:not([hidden])')].map(el=>el.textContent);
    return text.some(text=>text.includes("Branch")) && text.some(text=>text.includes("Root ancestor"));
  }),{timeout:10000});
  await browser.saveScreenshot(path.join(RUN,"sticky.png"));
  // GH #643: pins render their text (no raw **) and their bullets sit over the rows' bullets.
  const pins = await browser.execute(()=>[...document.querySelectorAll('.sticky-ancestor button:not([hidden])')].map(button=>{
    const own=button.closest(".sticky-ancestor").parentElement.querySelector(":scope > .block-main .bullet")?.getBoundingClientRect();
    const dot=button.querySelector(".sticky-ancestor-dot")?.getBoundingClientRect();
    return {text:button.textContent,strong:button.querySelector("strong")?.textContent??null,
      dx:own&&dot?Math.abs((own.left+own.width/2)-(dot.left+dot.width/2)):null};
  }));
  fs.writeFileSync(path.join(RUN,"sticky-pins.json"),JSON.stringify(pins,null,2));
  const rootPin=pins.find(p=>p.text.includes("Root ancestor"));
  if(!rootPin||rootPin.strong!=="ancestor"||pins.some(p=>p.text.includes("**"))) throw Error(`pinned parents show raw markup: ${JSON.stringify(pins)}`);
  if(pins.some(p=>p.dx===null||p.dx>1)) throw Error(`pinned bullets do not line up with their rows: ${JSON.stringify(pins)}`);
  await browser.execute(()=> {
    const button=[...document.querySelectorAll('.sticky-ancestor button:not([hidden])')].find(el=>el.textContent.includes("Root ancestor"));
    if(!button) throw Error("pinned root disappeared before activation");
    button.click();
  });
  await browser.waitUntil(()=>browser.execute(()=>document.querySelector(".main-content").scrollTop<1200),{timeout:10000});
  stage = "setting toggle";
  // The toolbar button, not the command palette: Enter could land before the
  // palette listed "Settings" on a loaded host (2 of 3 gate runs, 2026-10-18).
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForDisplayed({timeout:10000});
  const setting=await browser.$('[data-setting-label="Sticky parent bullets"] [role="switch"]');
  await setting.waitForDisplayed({timeout:10000});
  assert.equal(await setting.getAttribute("aria-checked"),"true");
  await setting.click();
  await browser.waitUntil(()=>browser.execute(()=>!document.querySelector(".sticky-ancestor")),{timeout:10000});
  await setting.click();
  await browser.keys(["Escape"]);
  await browser.execute(()=>document.querySelector(".main-content").scrollTop=1200);
  await browser.waitUntil(()=>browser.execute(()=>!!document.querySelector('.sticky-ancestor button:not([hidden])')),{timeout:10000});
  fs.writeFileSync(path.join(RUN,"receipt.json"),JSON.stringify({tested,app:APP,outcome:"PASS",before,after},null,2));
  console.log(`PASS: native editor context journey (${RUN})`);
} catch(error) {
  try { fs.writeFileSync(path.join(RUN,"scroll-observed.json"),JSON.stringify(await browser.execute(()=>({
    scroll:[...document.querySelectorAll(".main-content")].map(el=>({top:el.scrollTop,height:el.clientHeight,total:el.scrollHeight})),
    rows:[...document.querySelectorAll(".sticky-ancestor")].map(el=>({text:el.textContent,hidden:el.querySelector("button").hidden,top:el.getBoundingClientRect().top,rowTop:el.parentElement.querySelector(":scope > .block-main").getBoundingClientRect().top}))
  })),null,2)); } catch {}
  try { await browser?.saveScreenshot(path.join(RUN,"failure.png")); } catch {}
  try { fs.writeFileSync(path.join(RUN,"failure-dom.html"),await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(RUN,"failure.txt"),`Tested: ${tested}\nJourney: ${stage}\nExpected: stable code, selected macro host, pinned ancestors and a working toggle.\nObserved: ${error.stack}\nClassification: ambiguous until driver log and screenshot reviewed.\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid,"SIGKILL"); } catch {}
  fs.closeSync(log);
}

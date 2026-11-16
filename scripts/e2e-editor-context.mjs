// GH #643/#641/#510: native scroll/toggle, select an embed host, mixed code.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { installAccountHome } from "./lib/e2e-account-home.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = path.resolve(process.env.E2E_ARTIFACT_DIR || "artifacts/editor-2");
fs.mkdirSync(OUT, { recursive: true });
const RUN = fs.mkdtempSync(path.join(OUT, "native-")), GRAPH = path.join(RUN, "graph");
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), {recursive:true});
for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(RUN, dir));
installAccountHome(RUN);
const sourceId = "e2100000-0000-4000-8000-000000000510";
fs.writeFileSync(path.join(GRAPH,"logseq/config.edn"),"{}\n");
const source = `- AAA\n  \`\`\`js\n  BBB\n  CCC\n  \`\`\`\n  DDD\n  id:: ${sourceId}\n`;
fs.writeFileSync(path.join(GRAPH,"pages/Code source.md"), source);
for (const layout of ["desktop", "mobile"]) {
  fs.writeFileSync(path.join(GRAPH, `pages/Standalone ${layout}.md`), "- ```js\n  const x = 1;\n  ```\n- sibling\n");
}
fs.writeFileSync(path.join(GRAPH,"pages/Editor context.md"), `- Above embed\n- {{embed ((${sourceId}))}}\n- ((${sourceId}))\n- Root **ancestor**\n  - Branch\n${Array.from({length:180},(_,i)=>`    - Child ${i}`).join("\n")}\n`);
const now = new Date(), day = `${now.getFullYear()}_${String(now.getMonth()+1).padStart(2,"0")}_${String(now.getDate()).padStart(2,"0")}`;
fs.writeFileSync(path.join(GRAPH,`journals/${day}.md`), "- [[Editor context]]\n");
const APP = process.env.TINE_APP || path.join(ROOT,"target/release/tine");
const TD = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const port = Number(process.env.E2E_DRIVER_PORT || 4746);
// The existing native harness policy permits phone widths below the ordinary
// desktop window minimum. Layout still follows the real window's width.
const env = {...process.env,TINE_GRAPH:GRAPH,TINE_E2E_FORCE_MOBILE_DRAWERS:"1",XDG_DATA_HOME:path.join(RUN,"data"),XDG_CONFIG_HOME:path.join(RUN,"config"),XDG_CACHE_HOME:path.join(RUN,"cache"),XDG_STATE_HOME:path.join(RUN,"state"),WEBKIT_DISABLE_DMABUF_RENDERER:"1",WEBKIT_DISABLE_COMPOSITING_MODE:"1",LIBGL_ALWAYS_SOFTWARE:"1",GDK_BACKEND:"x11"};
// Photograph Xvfb directly: WebKitWebDriver's screenshot endpoint can stall
// after resizing to a narrow window. Images are artifacts, never the oracle.
const shot = name => execFileSync("import", ["-window", "root", path.join(RUN, name)], {env});
const log = fs.openSync(path.join(RUN,"driver.log"),"w");
const driver = spawn(TD,["--port",String(port),"--native-port",String(port+1),"--native-driver",process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],{env,stdio:["ignore",log,log],detached:true});
let browser, stage = "startup";
const tested = `${execFileSync("git", ["rev-parse", "HEAD"], {cwd: ROOT, encoding: "utf8"}).trim()} + working diff`;
try {
  browser = await remote({hostname:"127.0.0.1",port,path:"/",logLevel:"error",connectionRetryCount:2,connectionRetryTimeout:30000,capabilities:{browserName:"wry","wdio:enforceWebDriverClassic":true,"tauri:options":{application:APP}}});
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({timeout:30000});
  // GH #879: the literal reported merge path on a routed page, with both
  // desktop and narrow layouts. Persist both a changed language and prose
  // outside the wrapper through the application's guarded save path.
  for (const [layout, width] of [["desktop", 1200], ["mobile", 480]]) {
    stage = `standalone source: ${layout}`;
    await browser.setWindowSize(width, 900);
    await browser.waitUntil(() => browser.execute(mobile =>
      document.querySelector(".app-container")?.getAttribute("data-mobile-drawer-mode") === String(mobile), layout === "mobile"), {timeout:5000});
    await openPageByName(browser, `Standalone ${layout}`);
    if (layout === "mobile") {
      // Resizing an open desktop sidebar turns it into an open mobile drawer.
      // Dismiss it through its visible control before tapping page content.
      const close = await browser.$(".mobile-drawer-close");
      if (await close.isDisplayed()) await close.click();
      await browser.waitUntil(() => browser.execute(() =>
        document.querySelector(".app-container")?.getAttribute("data-active-drawer") === ""), {timeout:5000});
    }
    const contents = await browser.$$(".main-content .block-content");
    await contents.at(-1).click();
    const editor = await browser.$(".main-content textarea.block-editor");
    await editor.waitForExist({timeout:10000});
    await browser.keys(["Control", "a", "NULL"]);
    await browser.keys(["Backspace"]);
    assert.equal(await editor.getValue(), "", "the sibling is empty before merging");
    await browser.keys(["Backspace"]);
    await browser.waitUntil(async () => (await browser.$(".main-content textarea.block-editor").getValue()) === "```js\nconst x = 1;\n```", {
      timeout:5000, timeoutMsg:"Backspace into standalone code did not expose editable fences",
    });
    shot(`source-${layout}.png`);
    await browser.execute(() => document.querySelector(".main-content textarea.block-editor").setSelectionRange(3, 5));
    await browser.keys("python");
    await browser.waitUntil(() => browser.execute(() => {
      const language = [...document.querySelectorAll(".autocomplete .ac-item")]
        .find(item => item.querySelector(".ac-label")?.textContent === "Python");
      if (!language) return false;
      language.dispatchEvent(new MouseEvent("mousedown", {button:0, bubbles:true, cancelable:true}));
      return true;
    }), {timeout:10000, timeoutMsg:"the Python language option was not offered"});
    await browser.waitUntil(async () => (await browser.$(".main-content textarea.block-editor").getValue()) === "const x = 1;", {timeout:5000});
    // Picking a language explicitly enters code editing; Escape returns to
    // the whole source, just like OG's code editor, before adding prose.
    await browser.keys(["Escape"]);
    await browser.waitUntil(async () => (await browser.$(".main-content textarea.block-editor").getValue()) === "```python\nconst x = 1;\n```", {timeout:5000});
    await browser.keys(["Control", "End", "NULL"]);
    // Shift+Enter appends a newline within the raw block rather than splitting.
    await browser.keys(["Shift", "Enter", "NULL"]);
    await browser.keys(`after ${layout}`);
    shot(`source-edited-${layout}.png`);
    await browser.keys(["Escape"]);
    await waitForFileText(path.join(GRAPH, `pages/Standalone ${layout}.md`), text =>
      text.replace(/^[ \t]+/gm, "").includes(`\`\`\`python\nconst x = 1;\n\`\`\`\nafter ${layout}`));
  }
  await browser.setWindowSize(1200, 900);
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
  shot("mixed-edit.png");
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
  shot("sticky.png");
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
  try { shot("failure.png"); } catch {}
  try { fs.writeFileSync(path.join(RUN,"failure-dom.html"),await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(RUN,"failure.txt"),`Tested: ${tested}\nJourney: ${stage}\nExpected: editable standalone fences with language and surrounding prose saved, stable mixed code, selected macro host, pinned ancestors and a working toggle.\nObserved: ${error.stack}\nClassification: ambiguous until driver log and screenshot reviewed.\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid,"SIGKILL"); } catch {}
  fs.closeSync(log);
}

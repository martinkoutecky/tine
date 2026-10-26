// GH #700: real WebKitGTK copy buttons, stage timings and OS clipboard image.
// Run after npm run build + cargo build -p tine --features custom-protocol:
// Copy target/debug/tine to artifacts/gh700/tine-e2e after the custom-protocol
// build: concurrent cargo test can replace target/debug/tine with a dev build.
// Fixtures: artifacts/gh700/{photo.jpg,screenshot.png}, both 4000x3000.
// source scripts/env.sh; TINE_APP="$PWD/artifacts/gh700/tine-e2e" xvfb-run -a -s '-screen 0 1920x1080x24' node scripts/e2e-image-copy.mjs
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";

const root = process.cwd();
const fixtures = process.env.TINE_IMAGE_COPY_FIXTURES || path.join(root, "artifacts/gh700");
const out = path.join(fixtures, `native-${Date.now()}`);
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const name of ["photo.jpg", "screenshot.png"]) fs.copyFileSync(path.join(fixtures, name), path.join(graph, "assets", name));
// A real file above both the preview and bitmap budgets; sparse padding keeps
// fixture setup cheap while the unchanged original PNG remains the file source.
const largePath = path.join(graph, "assets", "large.png");
fs.copyFileSync(path.join(fixtures, "screenshot.png"), largePath);
fs.truncateSync(largePath, 65 * 1024 * 1024);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(graph, "pages/Image Copy.md"), "- ![photo](../assets/photo.jpg)\n- ![screenshot](../assets/screenshot.png)\n- ![large](../assets/large.png)\n- {{img ../assets/large.png}}\n- ![absent](../assets/absent.png)\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth()+1).padStart(2,"0")}_${String(now.getDate()).padStart(2,"0")}`;
fs.writeFileSync(path.join(graph, "journals", `${journal}.md`), "- [[Image Copy]]\n");
const env = { ...process.env, TINE_GRAPH: graph, WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
for (const key of ["DATA", "CONFIG", "CACHE"]) { env[`XDG_${key}_HOME`] = path.join(out, `xdg-${key}`); fs.mkdirSync(env[`XDG_${key}_HOME`], { recursive: true }); }
const xclip = process.env.TINE_XCLIP || "xclip";
env.PATH = `${path.dirname(xclip)}:${env.PATH}`;
const log = fs.openSync(path.join(out, "native.log"), "w");
const driver = spawn(path.join(process.env.CARGO_HOME, "bin/tauri-driver"), ["--port","4570","--native-port","4571","--native-driver","/usr/bin/WebKitWebDriver"], { env, stdio:["ignore",log,log], detached:true });
let browser;
let clipboardSeed = 0;
function seedClipboard() {
 const sentinel = `gh700-before-copy-${++clipboardSeed}`;
 execFileSync(xclip,["-selection","clipboard","-t","UTF8_STRING"],{input:sentinel,stdio:["pipe","ignore","ignore"],timeout:5000});
 assert.equal(execFileSync(xclip,["-o","-selection","clipboard","-t","UTF8_STRING"],{encoding:"utf8"}),sentinel);
 assert.ok(!execFileSync(xclip,["-o","-selection","clipboard","-t","TARGETS"],{encoding:"utf8"}).includes("image/png"));
}
function readCopiedImage() {
 const targets=execFileSync(xclip,["-o","-selection","clipboard","-t","TARGETS"],{encoding:"utf8"});
 assert.ok(targets.includes("image/png"),targets);
 const png=execFileSync(xclip,["-o","-selection","clipboard","-t","image/png"],{maxBuffer:64*1024*1024});
 assert.equal(png.subarray(0,8).toString("hex"),"89504e470d0a1a0a");
 assert.equal(png.readUInt32BE(16),4000);
 assert.equal(png.readUInt32BE(20),3000);
 return { targets, png };
}
try {
 const deadline = Date.now() + 15000;
 while (true) {
  if (driver.exitCode !== null) throw new Error(`tauri-driver exited before readiness (${driver.exitCode})`);
  const remaining = deadline - Date.now();
  if (remaining <= 0) throw new Error("tauri-driver did not become ready within 15 seconds");
  try {
   if ((await fetch("http://127.0.0.1:4570/status", { signal: AbortSignal.timeout(Math.min(1000, remaining)) })).ok) break;
  } catch {}
  await sleep(Math.min(100, Math.max(0, deadline - Date.now())));
 }
 browser = await remote({ hostname:"127.0.0.1",port:4570,path:"/",logLevel:"error",connectionRetryCount:0,connectionRetryTimeout:60000, capabilities:{browserName:"wry","wdio:enforceWebDriverClassic":true,"tauri:options":{application:process.env.TINE_APP || path.join(root,"target/debug/tine")}} });
 console.log("session",browser.sessionId);
 await browser.setTimeout({ script: 30000, implicit: 0, pageLoad: 30000 });
 console.log("open seeded page");
 await browser.$(".page-ref*=Image Copy").waitForExist({timeout:30000});
 await browser.$(".page-ref*=Image Copy").click();
 await browser.$(".asset-action-btn").waitForExist({timeout:15000});
 console.log("seeded images ready");
 const timings=[];
 for (const index of [0,1]) {
  console.log("measure previous pipeline",index);
  // Observe the old pipeline's stages in this actual WebKitGTK process.
  const old = await browser.executeAsync((idx, done) => {
   (async () => {
    const img = [...document.querySelectorAll(".page-blocks img")][idx];
    const t=performance.now(); await img.decode(); const decode=performance.now();
    const c=document.createElement("canvas"); c.width=img.naturalWidth;c.height=img.naturalHeight;c.getContext("2d").drawImage(img,0,0);const draw=performance.now();
    const blob=await new Promise(r=>c.toBlob(r,"image/png"));const encode=performance.now();const bytes=new Uint8Array(await blob.arrayBuffer());const buffer=performance.now();
    let s="";for(let i=0;i<bytes.length;i+=32768)s+=String.fromCharCode(...bytes.subarray(i,i+32768));const b64=btoa(s);const base64=performance.now();JSON.stringify({bytesB64:b64});
    const json=performance.now();
    const fetched=await fetch(img.src); const source=await fetched.blob(); const fetchEnd=performance.now();
    await source.arrayBuffer();
    done({decode:decode-t,draw:draw-decode,pngEncode:encode-draw,arrayBuffer:buffer-encode,base64:base64-buffer,json:json-base64,pngBytes:bytes.length,base64Bytes:b64.length,sourceBytes:source.size,sourceType:source.type,sourceFetch:fetchEnd-json,sourceBuffer:performance.now()-fetchEnd});
   })().catch(e=>done({stageError:String(e)}));
  }, index);
  assert.ok(!old.stageError, old.stageError);
  console.log("old stages",JSON.stringify(old));
  seedClipboard();
  const native = await browser.executeAsync((idx, done) => {
   (async () => {
    const source=await (await fetch([...document.querySelectorAll(".page-blocks img")][idx].src)).blob();
    const bytes=new Uint8Array(await source.arrayBuffer());
    const start=performance.now();
    await window.__TAURI_INTERNALS__.invoke("copy_image_to_clipboard",bytes);
    done({bytes:bytes.length,ms:performance.now()-start});
   })().catch(e=>done({stageError:String(e)}));
  },index);
  assert.ok(!native.stageError,native.stageError);
  assert.equal(native.bytes,old.sourceBytes,"original compressed bytes cross IPC");
  readCopiedImage();
  console.log("native raw IPC and clipboard",JSON.stringify(native));
  const buttons = await browser.$$('button[title="Copy image"]');
  await (await browser.$$(".page-blocks img"))[index].scrollIntoView();
  await (await browser.$$(".page-blocks img"))[index].moveTo();
  await browser.execute(() => {
   globalThis.__imageCopyProgress = false;
   const observer = new MutationObserver(() => {
    if ([...document.querySelectorAll(".toast")].some(t => t.textContent.includes("Copying image"))) globalThis.__imageCopyProgress = true;
   });
   observer.observe(document.body,{childList:true,subtree:true});
   globalThis.__imageCopyObserver = observer;
  });
  seedClipboard();
  const start=performance.now();
  await buttons[index].click();
  console.log("copy clicked",index);
  await browser.waitUntil(()=>browser.execute(()=>[...document.querySelectorAll(".toast")].some(t=>t.textContent.includes("Image copied"))),{timeout:30000});
  assert.ok(await browser.execute(() => { globalThis.__imageCopyObserver.disconnect(); return globalThis.__imageCopyProgress; }), "copy progress was visible");
  const { targets, png } = readCopiedImage();
  assert.ok(targets.includes("text/uri-list"), targets);
  assert.ok(targets.includes("x-special/gnome-copied-files"), targets);
  const uris = execFileSync(xclip,["-o","-selection","clipboard","-t","text/uri-list"],{encoding:"utf8"});
  assert.equal(uris.trim(), new URL(`file://${path.join(graph,"assets",index === 0 ? "photo.jpg" : "screenshot.png")}`).href);
  assert.deepEqual(fs.readFileSync(path.join(graph,"assets",index === 0 ? "photo.jpg" : "screenshot.png")), fs.readFileSync(path.join(fixtures,index === 0 ? "photo.jpg" : "screenshot.png")));
  timings.push({index,old,native,newClickToClipboardMs:performance.now()-start,targets,pngBytes:png.length});
  // Wait for the prior success toast to leave before the next copy.
  await browser.waitUntil(()=>browser.execute(()=>![...document.querySelectorAll(".toast")].some(t=>t.textContent.includes("Image copied"))),{timeout:8000});
 }
 // The lightbox shares the same transport and must also reach the OS clipboard.
 await (await browser.$$(".page-blocks img"))[0].click();
 seedClipboard();
 await browser.$(".lightbox-copy").click();
 await browser.waitUntil(()=>browser.execute(()=>[...document.querySelectorAll(".toast")].some(t=>t.textContent.includes("Image copied"))),{timeout:30000});
 const lightboxCopy = readCopiedImage();
 assert.ok(lightboxCopy.targets.includes("text/uri-list"), lightboxCopy.targets);
 assert.equal(execFileSync(xclip,["-o","-selection","clipboard","-t","text/uri-list"],{encoding:"utf8"}).trim(), new URL(`file://${path.join(graph,"assets","photo.jpg")}`).href);
 await browser.keys("Escape");
 await browser.$(".inline-image-unavailable").waitForExist({timeout:15000});
 const unavailableImages = await browser.$$(".inline-image-wrap:has(.inline-image-unavailable)");
 assert.equal(unavailableImages.length,2,"Markdown and macro refused previews");
 for (const unavailable of unavailableImages) {
  await unavailable.scrollIntoView();
  await unavailable.moveTo();
  const largeCopy = await unavailable.$('button[title="Copy image"]');
  seedClipboard();
  await largeCopy.click();
  await browser.waitUntil(async () => {
   const targets = execFileSync(xclip,["-o","-selection","clipboard","-t","TARGETS"],{encoding:"utf8"});
   return targets.includes("text/uri-list");
  },{timeout:15000});
  const largeTargets = execFileSync(xclip,["-o","-selection","clipboard","-t","TARGETS"],{encoding:"utf8"});
  assert.ok(!largeTargets.includes("image/png"),largeTargets);
  assert.equal(execFileSync(xclip,["-o","-selection","clipboard","-t","text/uri-list"],{encoding:"utf8"}).trim(),new URL(`file://${largePath}`).href);
  assert.equal(fs.statSync(largePath).size,65*1024*1024);
 }
 // An outside replacement revokes the inline blob; the already-open viewer
 // must still copy the graph file without fetching its stale preview URL.
 await (await browser.$$(".page-blocks img"))[0].scrollIntoView();
 await (await browser.$$(".page-blocks img"))[0].click();
 fs.copyFileSync(largePath,path.join(graph,"assets","photo.jpg"));
 await browser.waitUntil(()=>browser.execute(()=>[...document.querySelectorAll(".inline-image-unavailable")].some(el=>el.textContent.includes("photo"))),{timeout:15000});
 seedClipboard();
 await browser.$(".lightbox-copy").click();
 await browser.waitUntil(()=>execFileSync(xclip,["-o","-selection","clipboard","-t","TARGETS"],{encoding:"utf8"}).includes("text/uri-list"),{timeout:15000});
 const refreshedTargets = execFileSync(xclip,["-o","-selection","clipboard","-t","TARGETS"],{encoding:"utf8"});
 assert.ok(!refreshedTargets.includes("image/png"),refreshedTargets);
 assert.equal(execFileSync(xclip,["-o","-selection","clipboard","-t","text/uri-list"],{encoding:"utf8"}).trim(),new URL(`file://${path.join(graph,"assets","photo.jpg")}`).href);
 await browser.keys("Escape");
 assert.equal(await browser.$(".inline-image-missing").getText(),"🖼 absent");
 fs.writeFileSync(path.join(out,"native-stages.json"),JSON.stringify(timings,null,2));
 await browser.saveScreenshot(path.join(out,"copy-success.png"));
 console.log("PASS inline JPEG/PNG and lightbox copy offer file and pixels; Markdown and macro refused 65 MiB previews offer file only; open viewer retains file copy after live refresh; missing asset retains placeholder",JSON.stringify(timings));
} catch (error) {
 await browser?.saveScreenshot(path.join(out,"failure.png")).catch(()=>{});
 fs.writeFileSync(path.join(out,"failure.json"), JSON.stringify({
  commit: execFileSync("git",["rev-parse","HEAD"],{encoding:"utf8"}).trim(),
  journey: "inline JPEG/PNG, lightbox, Markdown/macro refused previews and open-viewer live refresh copy",
  expected: "loaded images offer original asset URI plus image/png; refused previews and the refreshed open viewer offer their original file only; missing assets retain their placeholder",
  observed: String(error), screenshot: "failure.png", log: "native.log",
  classification: "ambiguous",
 },null,2));
 throw error;
} finally {
 await browser?.deleteSession().catch(()=>{});
 try { process.kill(-driver.pid,"SIGKILL"); } catch {}
 fs.closeSync(log);
}

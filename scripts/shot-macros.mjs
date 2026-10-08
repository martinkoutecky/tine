// Screenshot the new OG macros on the kitchen-sink page (user :macros, twitter,
// vimeo, bilibili, youtube-timestamp, cloze, zotero).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
const PORT = 5205;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function wait(url){for(let i=0;i<60;i++){try{if((await fetch(url)).ok)return}catch{}await sleep(250)}throw new Error("no server")}
try{
  await wait(`http://localhost:${PORT}/`);
  const b = await chromium.launch();
  const p = await b.newPage({viewport:{width:900,height:1500}});
  const errs=[]; p.on("pageerror",e=>errs.push(String(e))); p.on("console",m=>m.type()==="error"&&errs.push(m.text()));
  await p.goto(`http://localhost:${PORT}/`);
  await p.waitForSelector(".ls-block",{timeout:5000});
  await p.keyboard.press("Control+k");
  await p.waitForSelector(".switcher-input",{timeout:3000});
  await p.locator(".switcher-input").fill("kitchen"); await sleep(400);
  await p.locator(".switcher-row").first().click();
  await p.waitForSelector(".cloze",{timeout:5000});
  await sleep(700);
  // Find the user-macro block and screenshot it + its neighbors for the macros set.
  const ublk = p.locator(".ls-block", { hasText: "User macro" }).first();
  await ublk.scrollIntoViewIfNeeded();
  await sleep(300);
  await p.screenshot({ path: "screenshots/macros.png", clip: await (async()=>{const bb=await ublk.boundingBox(); return {x:0,y:Math.max(0,bb.y-150),width:900,height:340};})() });
  console.log("user-macro text:", (await ublk.textContent())?.trim());
  console.log("cloze count:", await p.locator(".cloze").count(), "yt-ts:", await p.locator(".youtube-ts").count(), "zotero:", await p.locator(".zotero-ref").count(), "iframes:", await p.locator("iframe.embed-iframe").count());
  console.log(errs.length ? "ERRORS:\n"+errs.join("\n") : "no console errors");
  await b.close();
}finally{server.kill()}

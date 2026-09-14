import { createServer } from "vite";
import { chromium } from "playwright";
import { mkdir, writeFile } from "node:fs/promises";
import { realpathSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),"..");
const out=process.env.TINE_PROBE_OUT ?? path.join(root,"screenshots/default-task-css");
await mkdir(out,{recursive:true});
const server=await createServer({root,server:{host:"127.0.0.1",port:0,fs:{allow:[root,realpathSync(path.join(root,"node_modules/inter-ui")),realpathSync(path.join(root,"node_modules/@fontsource-variable/noto-emoji"))]}}});
await server.listen();
const browser=await chromium.launch({args:["--no-sandbox"]});
const failures=[];const results=[];
const check=(ok,message)=>{if(!ok)failures.push(message)};
try {
 for(const mode of ["light","dark","custom"]) {
  const page=await browser.newPage({viewport:{width:1100,height:1000}});
  const errors=[];page.on("pageerror",e=>errors.push(e.message));
  await page.goto(`${server.resolvedUrls.local[0]}scripts/fixtures/default-task-css/index.html`);
  await page.locator('[data-block-id="task-todo"] .block-task-checkbox').waitFor();
  await page.evaluate(mode=>{
   document.documentElement.dataset.theme=mode==="dark"?"dark":"light";
   if(mode==="custom") {
    const style=document.createElement("style");style.id="tine-custom-css";
    style.textContent='html[data-theme="light"] { --ls-primary-text-color: #164e63; --ls-title-text-color: #831843; --marker-todo: #a21caf; --marker-done: #047857; }';
    document.head.append(style);
   }
  },mode);
  await page.evaluate(()=>document.fonts.ready);
  const read=await page.evaluate(()=>{
   const c=e=>getComputedStyle(e);
   const title=document.querySelector(".page-title");
   const contents=[...document.querySelectorAll(".ls-block > .block-main > .block-content-wrapper > .block-content")];
   const normal=contents.find(e=>!e.classList.contains("done"));
   const completed=contents.find(e=>e.classList.contains("done"));
   const directBody=[...completed.children].find(e=>e.matches("[data-so]"));
   const marker=completed.querySelector(".block-marker");
   const nested=document.querySelector("#nested-reference-probe .block-marker");
   const states=[...document.querySelectorAll(".block-task-checkbox")].map(e=>({id:e.closest("[data-block-id]").dataset.blockId,color:c(e).borderColor,border:c(e).borderTopWidth,background:c(e).backgroundColor,marker:c(e.nextElementSibling).color,checked:e.classList.contains("checked")}));
   return {fonts:[...document.fonts].filter(f=>f.status==="loaded").map(f=>f.family),title:c(title).color,titleLine:parseFloat(c(title).lineHeight)/parseFloat(c(title).fontSize),primary:c(normal).color,completed:{color:c(completed).color,opacity:c(completed).opacity,decoration:c(completed).textDecorationLine,bodyDecoration:directBody?c(directBody).textDecorationLine:null,labelDecoration:c(marker).textDecorationLine},nested:{display:c(nested).display,decoration:c(nested).textDecorationLine},states};
  });
  check(read.fonts.includes("Inter"),`${mode}: bundled Inter loaded`);
  check(read.titleLine===1.5,`${mode}: page title line height`);
  check(mode==="custom" ? read.primary==="rgb(22, 78, 99)" && read.title==="rgb(131, 24, 67)" : read.primary===read.title,`${mode}: primary default/custom override`);
  check(read.completed.color===read.primary && read.completed.opacity==="0.5",`${mode}: completed text and dimming`);
  check(read.completed.decoration==="none" && read.completed.labelDecoration==="none",`${mode}: parent and label unstruck`);
  check(read.completed.bodyDecoration==="line-through",`${mode}: body retains strike`);
  check(read.nested.decoration==="none" && read.nested.display==="inline-block",`${mode}: nested reference marker isolates inherited strike`);
  for(const state of read.states) {
   check(state.checked ? state.border==="0px" && state.background===state.marker : state.color===state.marker,`${mode}: ${state.id} checkbox follows marker`);
  }
  const hover=[];
  for(const id of ["todo","doing","later","now","waiting","wait","started","in-progress"]) {
   const box=page.locator(`[data-block-id="task-${id}"] .block-task-checkbox`);await box.hover();
   const color=await box.evaluate(e=>{
    const expected=getComputedStyle(e.nextElementSibling).color;
    const sample=document.createElement("span");sample.style.backgroundColor=`color-mix(in srgb, ${expected} 20%, transparent)`;document.body.append(sample);
    const expectedBackground=getComputedStyle(sample).backgroundColor;sample.remove();
    return {background:getComputedStyle(e).backgroundColor,border:getComputedStyle(e).borderColor,expected,expectedBackground};
   });
   check(color.background===color.expectedBackground && color.border===color.expected,`${mode}: ${id} hover feedback`);hover.push({id,...color});
  }
  await page.mouse.move(0,0);
  check(errors.length===0,`${mode}: page errors ${errors.join(", ")}`);
  await page.screenshot({path:path.join(out,`${mode}.png`),fullPage:true});
  results.push({mode,...read,hover,errors});await page.close();
 }
 await writeFile(path.join(out,"results.json"),JSON.stringify({results,failures},null,2));
 console.log(JSON.stringify({checks:"default CSS across light, dark and custom theme",failures},null,2));
 if(failures.length)process.exitCode=1;
} finally {await browser.close();await server.close();}

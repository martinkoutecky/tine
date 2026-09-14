import {createServer} from "vite";
import {chromium} from "playwright";
import {mkdir,writeFile} from "node:fs/promises";
import {realpathSync} from "node:fs";
import path from "node:path";
import {fileURLToPath} from "node:url";
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),"..");
const out=process.env.TINE_PROBE_OUT??path.join(root,"screenshots/mobile-toolbar");await mkdir(out,{recursive:true});
const server=await createServer({root,server:{host:"127.0.0.1",port:0,fs:{allow:[root,realpathSync(path.join(root,"node_modules/inter-ui"))]}}});await server.listen();
const browser=await chromium.launch({args:["--no-sandbox"]});const failures=[],results=[];
try{
 const page=await browser.newPage({viewport:{width:390,height:720},isMobile:true,hasTouch:true});
 await page.goto(`${server.resolvedUrls.local[0]}scripts/fixtures/mobile-toolbar/index.html`);await page.waitForFunction(()=>window.toolbarProbe);
 for(const [kind,label,scroll] of [["indent","Indent",0],["outdent","Outdent",0],["move","Move block up",0],["move","Move block down",0],["move","Move block up",70],["indent","Indent",42]]){
  await page.evaluate(kind=>window.toolbarProbe.setup(kind),kind);
  await page.locator("textarea.block-editor").waitFor();
  await page.evaluate(scroll=>{const strip=document.querySelector(".mobile-keyboard-toolbar-strip");strip.scrollLeft=scroll;window.beforeToolbar=document.querySelector("[data-mobile-keyboard-toolbar]")},scroll);
  const box=await page.getByRole("button",{name:label,exact:true}).boundingBox();
  await page.touchscreen.tap(box.x+box.width/2,box.y+box.height/2);
  await page.waitForTimeout(40);
  const result=await page.evaluate(()=>({commands:[...window.toolbarProbe.commands],trace:[...window.toolbarProbe.trace],activeEditor:document.activeElement?.matches("textarea.block-editor")&&document.activeElement.closest(".ls-block")?.dataset.blockId==="current",sameToolbar:window.beforeToolbar===document.querySelector("[data-mobile-keyboard-toolbar]"),scroll:document.querySelector(".mobile-keyboard-toolbar-strip")?.scrollLeft,parent:window.toolbarProbe.doc.byId.current?.parent,roots:window.toolbarProbe.doc.pages.find(p=>p.name==="Toolbar probe")?.roots}));
  const expectedParent=kind==="indent"?"parent":kind==="outdent"?"grand":null;
  const expectedRoots=label==="Move block up"?["a","current","b","d","e"]:label==="Move block down"?["a","b","d","current","e"]:null;
  if(result.parent!==expectedParent || (expectedRoots && JSON.stringify(result.roots)!==JSON.stringify(expectedRoots)))failures.push(`${kind}/${label}/${scroll}: wrong structural result`);
  if(!result.activeEditor)failures.push(`${kind}/${label}/${scroll}: focused editor was lost`);
  if(result.commands.length!==1)failures.push(`${kind}/${label}/${scroll}: ${result.commands.join(",")}`);
  if(!result.sameToolbar||result.scroll!==scroll)failures.push(`${kind}/${label}/${scroll}: toolbar replaced or scroll reset`);
  results.push({kind,label,requestedScroll:scroll,...result});
 }
 // Two independent gestures back-to-back must both act, even when the first
 // WebView compatibility click would have fallen inside the old 400ms window.
 await page.evaluate(()=>window.toolbarProbe.setup("indent"));
 const indentBox=await page.getByRole("button",{name:"Indent",exact:true}).boundingBox();
 await page.touchscreen.tap(indentBox.x+indentBox.width/2,indentBox.y+indentBox.height/2);
 await page.touchscreen.tap(indentBox.x+indentBox.width/2,indentBox.y+indentBox.height/2);
 const rapid=await page.evaluate(()=>({commands:[...window.toolbarProbe.commands],parent:window.toolbarProbe.doc.byId.current?.parent}));
 if(rapid.commands.length!==2||rapid.parent!=="previous")failures.push("rapid independent taps were dropped or duplicated");
 results.push({kind:"rapid",...rapid});
 // Real keyboard selection must survive the same native blur/remount as touch.
 for(const kind of ["indent","outdent"]) for(const direction of ["forward","backward"]){
  await page.evaluate(kind=>window.toolbarProbe.setup(kind),kind);
  await page.keyboard.press(direction==="forward"?"Home":"End");
  for(let i=0;i<3;i++)await page.keyboard.press(direction==="forward"?"Shift+ArrowRight":"Shift+ArrowLeft");
  const before=await page.locator("textarea.block-editor").evaluate(e=>({start:e.selectionStart,end:e.selectionEnd,direction:e.selectionDirection}));
  await page.keyboard.press(kind==="indent"?"Tab":"Shift+Tab");
  await page.waitForTimeout(40);
  const after=await page.evaluate(()=>{const e=document.activeElement;return e?.matches("textarea.block-editor")?{start:e.selectionStart,end:e.selectionEnd,direction:e.selectionDirection}:null});
  if(!after||JSON.stringify(before)!==JSON.stringify(after)||before.direction!==direction||before.end-before.start!==3)failures.push(`${kind}/${direction}: keyboard selection or focus lost`);
  results.push({kind:"keyboard-selection",operation:kind,direction,before,after});
 }
 for(const label of ["Move block up","Move block down"])for(const direction of ["forward","backward"]){
  await page.evaluate(()=>window.toolbarProbe.setup("move"));await page.keyboard.press(direction==="forward"?"Home":"End");
  for(let i=0;i<3;i++)await page.keyboard.press(direction==="forward"?"Shift+ArrowRight":"Shift+ArrowLeft");
  const before=await page.locator("textarea.block-editor").evaluate(e=>({start:e.selectionStart,end:e.selectionEnd,direction:e.selectionDirection}));
  const box=await page.getByRole("button",{name:label,exact:true}).boundingBox();await page.touchscreen.tap(box.x+box.width/2,box.y+box.height/2);await page.waitForTimeout(40);
  const after=await page.evaluate(()=>{const e=document.activeElement;return e?.matches("textarea.block-editor")?{start:e.selectionStart,end:e.selectionEnd,direction:e.selectionDirection}:null});
  if(JSON.stringify(before)!==JSON.stringify(after))failures.push(`${label}/${direction}: move lost selection or focus`);results.push({kind:"move-selection",label,direction,before,after});
 }
 // A horizontal swipe over buttons belongs to scrolling, not activation.
 await page.evaluate(()=>window.toolbarProbe.setup("move"));
 const strip=await page.locator(".mobile-keyboard-toolbar-strip").boundingBox();
 const cdp=await page.context().newCDPSession(page);
 const y=strip.y+strip.height/2;
 await cdp.send("Input.dispatchTouchEvent",{type:"touchStart",touchPoints:[{x:270,y}]});
 for(const x of [230,190,150,110,70])await cdp.send("Input.dispatchTouchEvent",{type:"touchMove",touchPoints:[{x,y}]});
 await cdp.send("Input.dispatchTouchEvent",{type:"touchEnd",touchPoints:[]});
 const swipe=await page.evaluate(()=>({commands:[...window.toolbarProbe.commands],scroll:document.querySelector(".mobile-keyboard-toolbar-strip").scrollLeft,trace:[...window.toolbarProbe.trace]}));
 if(swipe.commands.length||swipe.scroll<=0)failures.push("swipe activated a command or failed to scroll");
 results.push({kind:"swipe",...swipe});await cdp.detach();
 await page.screenshot({path:path.join(out,"toolbar.png")});await writeFile(path.join(out,"results.json"),JSON.stringify({results,failures},null,2));console.log(JSON.stringify({failures},null,2));if(failures.length)process.exitCode=1;
}finally{await browser.close();await server.close();}

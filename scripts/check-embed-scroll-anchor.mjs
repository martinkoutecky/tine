import {createServer} from "vite";
import {chromium} from "playwright";
import {mkdir,writeFile} from "node:fs/promises";
import {realpathSync} from "node:fs";
import path from "node:path";
import {fileURLToPath} from "node:url";
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),"..");
const out=process.env.TINE_PROBE_OUT??path.join(root,"screenshots/embed-scroll-anchor");await mkdir(out,{recursive:true});
const server=await createServer({root,server:{host:"127.0.0.1",port:0,fs:{allow:[root,realpathSync(path.join(root,"node_modules/inter-ui"))]}}});await server.listen();
const browser=await chromium.launch({args:["--no-sandbox"]});const results=[],failures=[];
try{
 const page=await browser.newPage({viewport:{width:900,height:700}});
 await page.goto(`${server.resolvedUrls.local[0]}scripts/fixtures/embed-input/index.html`);await page.waitForFunction(()=>window.embedProbe);
 const setup=async(position="above",large=false)=>{
  await page.evaluate(([position,large])=>window.embedProbe.setup(false,position,large),[position,large]);
  if(position!=="none")await page.locator('.embed-block [data-block-id="child-two"]').waitFor();
  await page.evaluate(()=>{const main=document.querySelector('main');main.style.height='400px';main.style.padding='32px';document.querySelector('main > [data-block-id="tail"]').style.marginBottom='1000px';main.scrollTop=50});
  await page.locator('main > [data-block-id="source"] > .block-main .block-content').click();await page.keyboard.press('Control+End');
 };
 const measure=()=>page.evaluate(async()=>{const e=document.activeElement,r=e.getBoundingClientRect();const points=e.matches('textarea.block-editor')?(await import('/src/editor/caretRows.ts')).textareaCaretPoints(e):null;return {top:r.top,caretY:points?r.top+points[e.selectionEnd].y-e.scrollTop:null,scroll:document.querySelector('main').scrollTop,active:e.closest('.ls-block')?.dataset.blockId,mirrorHeight:document.querySelector('.embed-block')?.getBoundingClientRect().height??0}});
 for(const [position,large] of [["above",false],["below",false],["none",false],["above",true],["none",true]]){
  await setup(position,large);const before=await measure();await page.keyboard.insertText('\nOne\nTwo\nThree\nFour');await page.waitForTimeout(80);const added=await measure();await page.keyboard.press('Control+z');await page.waitForTimeout(80);const removed=await measure();
  if((!large&&(Math.abs(before.top-added.top)>1||Math.abs(before.top-removed.top)>1))||added.active!=="source"||removed.active!=="source")failures.push(`${position}/${large}: editor origin moved`);
  if(!large&&position!=="above"&&Math.abs(before.scroll-added.scroll)>1)failures.push(`${position}: unrelated scroll compensation`);
  results.push({position,large,before,added,removed});
 }
 // Large textarea native caret reveal is a separate owner. Compare the same
 // edit without a mirror so its own small reveal is not falsely called a jump.
 const largeMirror=results.find(r=>r.large&&r.position==="above"),largeControl=results.find(r=>r.large&&r.position==="none");
 for(const phase of ["added","removed"])if(Math.abs((largeMirror[phase].top-largeMirror.before.top)-(largeControl[phase].top-largeControl.before.top))>1)failures.push(`large/${phase}: mirror adds displacement beyond ordinary editor`);
 const pauseFrames=()=>page.evaluate(()=>{window.savedRaf=requestAnimationFrame;window.pendingFrames=[];window.requestAnimationFrame=callback=>{window.pendingFrames.push(callback);return window.pendingFrames.length}});
 const resumeFrames=()=>page.evaluate(()=>{window.requestAnimationFrame=window.savedRaf;for(const callback of window.pendingFrames)requestAnimationFrame(callback);window.pendingFrames=[]});
 await setup();const coalescedBefore=await measure();await pauseFrames();await page.keyboard.insertText('\nFirst');await page.keyboard.insertText('\nSecond');await resumeFrames();await page.waitForTimeout(80);const coalescedAfter=await measure();
 if(Math.abs(coalescedBefore.top-coalescedAfter.top)>1)failures.push('two commits in one frame lost their original anchor');results.push({kind:'coalesced',before:coalescedBefore,after:coalescedAfter});
 for(const interruption of ["wheel","focus","disconnect"]){
  await setup();await pauseFrames();await page.keyboard.insertText('\nOne\nTwo\nThree\nFour');
  if(interruption==="wheel"){await page.mouse.move(400,200);await page.mouse.wheel(0,80);await page.waitForTimeout(30)}
  else if(interruption==="focus")await page.locator('main > [data-block-id="after"] > .block-main .block-content').click();
  else await page.keyboard.press('Escape');
  const interrupted=await measure();await resumeFrames();await page.waitForTimeout(80);const after=await measure();
  if(Math.abs(after.scroll-interrupted.scroll)>1)failures.push(`${interruption}: pending anchor counteracted interruption`);
  if(interruption==="focus"&&after.active!=="after")failures.push('focus owner was lost');
  results.push({interruption,interrupted,after});
 }
 await writeFile(path.join(out,'results.json'),JSON.stringify({results,failures},null,2));console.log(JSON.stringify({failures},null,2));if(failures.length)process.exitCode=1;
}finally{await browser.close();await server.close()}

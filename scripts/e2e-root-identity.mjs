// L9-01/L9-02: moved live root -> device custody -> explicit recovery at
// the new location, then a confirmed restore from an unchanged old-path zip.
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { answerNativeDialog } from "./lib/e2e-native-dialog.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { APP_ID } from "./lib/app-identity.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";

const root = path.resolve(import.meta.dirname,"..");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root,"artifacts/og-root-identity-r3/native"));
fs.mkdirSync(artifacts,{ recursive:true });
const run = fs.mkdtempSync(path.join(artifacts,"run-"));
const old = path.join(run,"old/notes"), moved = path.join(run,"new/notes");
for (const dir of ["pages","journals","logseq","assets"]) fs.mkdirSync(path.join(old,dir),{ recursive:true });
for (const dir of ["data","config","cache"]) fs.mkdirSync(path.join(run,dir));
const ownerFixture = installAccountHome(run);
fs.mkdirSync(path.dirname(moved),{ recursive:true });
fs.writeFileSync(path.join(old,"pages/A.md"),"- before move\n");
fs.writeFileSync(path.join(old,"logseq/config.edn"),"{}\n");
// cargo test also produces target/debug/tine without custom-protocol. Pin this
// run's built app across crash/relaunch instead of racing that shared output.
const app = path.join(run,"tine");
fs.copyFileSync(process.env.TINE_APP || path.join(root,"target/debug/tine"),app);
fs.chmodSync(app,0o700);
const td = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME,"bin/tauri-driver");
let launches=0;
const unusedPort=()=>new Promise((resolve,reject)=>{
  const server=net.createServer(); server.on("error",reject);
  server.listen(0,"127.0.0.1",()=>{
    const port=server.address().port;
    server.close(error=>error ? reject(error) : resolve(port));
  });
});
const env = { ...process.env, XDG_DATA_HOME:path.join(run,"data"),XDG_CONFIG_HOME:path.join(run,"config"),XDG_CACHE_HOME:path.join(run,"cache"),
  TINE_ROOT_IDENTITY_RUN:run, WEBKIT_DISABLE_DMABUF_RENDERER:"1",WEBKIT_DISABLE_COMPOSITING_MODE:"1",LIBGL_ALWAYS_SOFTWARE:"1",GDK_BACKEND:"x11" };
const data = path.join(run,"data",APP_ID);
const testedCommit = execFileSync("git", ["rev-parse","HEAD"],{ cwd:root,encoding:"utf8" }).trim();
const log = fs.openSync(path.join(run,"driver.log"),"w");
let driver,browser,generation,stage="launch";
const passed=[];
const assert = (condition,message) => { if (!condition) throw new Error(message); };
const invoke = async (command,args={}) => {
  const answer = await browser.executeAsync((command,args,generation,done) => {
    window.__TAURI_INTERNALS__.invoke(command,{ ...args,bindingGeneration:generation })
      .then(value=>done({ok:true,value}),error=>done({ok:false,error:String(error)}));
  },command,args,generation);
  assert(answer.ok,`${command}: ${answer.error}`); return answer.value;
};
function appPids() {
  return fs.readdirSync("/proc").filter(pid=>/^\d+$/.test(pid)).filter(pid=>{
    try { return fs.readFileSync(`/proc/${pid}/environ`,"utf8").split("\0").includes(`TINE_ROOT_IDENTITY_RUN=${run}`)
      && path.basename(fs.readlinkSync(`/proc/${pid}/exe`)) === path.basename(app); }
    catch { return false; }
  }).map(Number);
}
async function crash() {
  if (driver) { try { process.kill(-driver.pid,"SIGKILL"); } catch {} }
  for (const pid of appPids()) { try { process.kill(pid,"SIGKILL"); } catch {} }
  for (let n=0;n<100 && appPids().length;n++) await sleep(100);
  assert(appPids().length===0,"the old host must release its lock before relaunch");
  browser=undefined; driver=undefined;
}
async function launch(graph) {
  // A killed driver's connections can remain in TIME_WAIT. Each relaunch gets
  // new listener ports instead of relying on immediate rebinding of its port.
  const port=process.env.E2E_DRIVER_PORT ? Number(process.env.E2E_DRIVER_PORT)+2*launches : await unusedPort();
  launches++;
  let nativePort=process.env.E2E_NATIVE_PORT ? Number(process.env.E2E_NATIVE_PORT)+2*(launches-1) : await unusedPort();
  while (nativePort===port) nativePort=await unusedPort();
  driver=spawn(td,["--port",String(port),"--native-port",String(nativePort),"--native-driver",process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
    { env:{...env,TINE_GRAPH:graph},stdio:["ignore",log,log],detached:true });
  await sleep(1200);
  browser=await remote({ hostname:"127.0.0.1",port,path:"/",logLevel:"error",connectionRetryCount:2,connectionRetryTimeout:30000,
    capabilities:{ browserName:"wry","wdio:enforceWebDriverClassic":true,"tauri:options":{application:app} } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout:30000 });
  // The shell mounts before asynchronous graph binding. Observe a real routed
  // page before reading the native lease, rather than racing startup with it.
  await openPageByName(browser,"A");
  await assertAppLease(ownerFixture);
  const binding=await browser.executeAsync((graph,done)=>window.__TAURI_INTERNALS__.invoke("load_graph",{path:graph}).then(done,error=>done({error:String(error)})),graph);
  assert(binding.kind==="already_current",`unexpected binding: ${JSON.stringify(binding)}`);
  generation=binding.binding_generation;
}
function draftsHold(text) {
  const drafts=path.join(data,"drafts-v2");
  if (!fs.existsSync(drafts)) return false;
  for (const dir of fs.readdirSync(drafts,{withFileTypes:true}).filter(e=>e.isDirectory())) {
    for (const file of fs.readdirSync(path.join(drafts,dir.name),{withFileTypes:true}).filter(e=>e.isFile())) {
      if (fs.readFileSync(path.join(drafts,dir.name,file.name)).includes(Buffer.from(text))) return true;
    }
  }
  return false;
}
const toastText=()=>browser.execute(()=>[...document.querySelectorAll(".toast-msg")].map(n=>n.textContent).join("\n"));
const clickNotice=async label=>browser.execute(label=>[...document.querySelectorAll('[data-testid="root-recovery-notice"] button')].find(b=>b.textContent===label)?.click(),label);
// WebKit's screenshot command can time out after a crash/relaunch even while
// DOM commands work. Capture the actual Xvfb display at the observed state.
const screenshot=name=>execFileSync("import", ["-window","root",path.join(run,name)],{env,timeout:10000});
try {
  await launch(old);
  await browser.waitUntil(async ()=>(await invoke("list_backups")).some(b=>b.status==="ok"),{timeout:30000,timeoutMsg:"initial launch snapshot never became available"});
  const originals = [];
  for (const dir of fs.readdirSync(path.join(data,"backups"),{withFileTypes:true}).filter(e=>e.isDirectory() && e.name.endsWith(".zips"))) {
    for (const file of fs.readdirSync(path.join(data,"backups",dir.name)).filter(n=>n.endsWith(".zip"))) {
      const name=path.join(data,"backups",dir.name,file); originals.push([name,fs.readFileSync(name)]);
    }
  }
  stage="moving an open folder keeps typed edits in device custody and never recreates the old root";
  await openPageByName(browser,"A");
  await browser.waitUntil(()=>browser.execute(()=>{
    const block=document.querySelector(".page .ls-block .block-content-wrapper");
    if (!(block instanceof HTMLElement)) return false;
    block.dispatchEvent(new MouseEvent("mousedown",{bubbles:true,cancelable:true,button:0}));
    block.dispatchEvent(new MouseEvent("mouseup",{bubbles:true,cancelable:true,button:0})); block.click();
    return document.activeElement instanceof HTMLTextAreaElement;
  }),{timeout:10000});
  await browser.keys(["End"]);
  fs.renameSync(old,moved);
  for (const key of " kept after move") await browser.keys([key]);
  await browser.waitUntil(async ()=>(await toastText()).includes("moved, replaced or disconnected"),{timeout:30000});
  await browser.waitUntil(()=>draftsHold("before move kept after move"),{timeout:30000,timeoutMsg:"typed text never reached a real durable draft record"});
  assert(!fs.existsSync(old),"saving recreated the vanished graph root");
  assert(fs.readFileSync(path.join(moved,"pages/A.md"),"utf8")==="- before move\n","a detached save changed the moved graph");
  screenshot("folder-moved.png"); passed.push(stage);
  await crash();

  stage="reopening the moved graph offers explicit orphan recovery, and Not now does not discard it";
  await launch(moved);
  await browser.$('[data-testid="root-recovery-notice"]').waitForExist({timeout:20000});
  assert((await browser.$('[data-testid="root-recovery-notice"]').getText()).includes(old),"notice did not name the old location");
  const recoveryNotice = await browser.$('[data-testid="root-recovery-notice"]').getText();
  assert(recoveryNotice.includes("moved or unavailable") && recoveryNotice.includes("disconnected drive"),"recovery notice did not explain disconnected storage");
  await clickNotice("Not now");
  await browser.$('[data-testid="root-recovery-notice"]').waitForExist({reverse:true,timeout:5000});
  assert(draftsHold("before move kept after move"),"Not now discarded the draft");
  await crash(); await launch(moved);
  await browser.$('[data-testid="root-recovery-notice"]').waitForExist({timeout:20000});
  screenshot("orphan-recovery.png");
  await clickNotice("Recover into this graph");
  await waitForFileText(path.join(moved,"pages/A.md"),text=>text==="- before move kept after move\n",{timeoutMs:30000});
  await browser.$('[data-testid="root-recovery-notice"]').waitForExist({reverse:true,timeout:15000});
  await browser.waitUntil(()=>browser.execute(()=>document.querySelector(".page")?.textContent.includes("before move kept after move")),{timeout:15000});
  assert(!(await browser.$$(".toast-error")).length,"recovery left an error instead of rebinding the editor");
  passed.push(stage);

  stage="old-location backups are visible and a confirmed restore restores verified original bytes";
  await browser.$('button[title^="Settings"]').click();
  await browser.execute(()=>[...document.querySelectorAll(".settings button")].find(b=>b.textContent?.trim()==="Backups & recovery")?.click());
  // Settings has several presentation wrappers; the tab is identified by its
  // visible label, and the old snapshot row by its domain path.
  await browser.execute(()=>[...document.querySelectorAll("button")].find(b=>b.textContent?.trim()==="Backups & recovery")?.click());
  await browser.waitUntil(()=>browser.execute(old=>[...document.querySelectorAll(".settings-backup-row")].some(row=>row.textContent.includes(old)),old),{timeout:15000});
  assert(await browser.execute(old=>[...document.querySelectorAll(".settings-backup-row")].some(row=>row.textContent.includes(old) && row.textContent.includes("moved or unavailable") && row.textContent.includes("disconnected drive")),old),"backup row did not explain disconnected storage");
  screenshot("backups-old-location.png");
  const restoreOld=()=>browser.execute(old=>{
    const row=[...document.querySelectorAll(".settings-backup-row")].find(row=>row.textContent.includes(old));
    [...row.querySelectorAll("button")].find(b=>b.textContent==="Restore").click();
  },old);
  await restoreOld(); await answerNativeDialog("no",{env});
  assert(fs.readFileSync(path.join(moved,"pages/A.md"),"utf8").includes("kept after move"),"declining restore changed the graph");
  await restoreOld(); await answerNativeDialog("yes",{env});
  await waitForFileText(path.join(moved,"pages/A.md"),text=>text==="- before move\n",{timeoutMs:30000});
  await browser.waitUntil(()=>browser.execute(()=>{
    const page=document.querySelector(".page");
    return page?.textContent.includes("before move") && !page.textContent.includes("kept after move");
  }),{timeout:15000,timeoutMsg:"the editor did not rebind to the restored graph"});
  assert(!(await browser.$$(".toast-error")).length,"restore left an error instead of rebinding the editor");
  for (const [name,bytes] of originals) assert(fs.readFileSync(name).equals(bytes),"an old snapshot zip was rewritten");
  passed.push(stage);
  fs.writeFileSync(path.join(run,"result.json"),JSON.stringify({outcome:"pass",testedCommit,passed},null,2));
  console.log(`PASS: ${passed.join("; ")}`);
} catch(error) {
  try { screenshot("failure.png"); } catch {}
  try { fs.writeFileSync(path.join(run,"failure-dom.html"),await browser.execute(()=>document.documentElement.outerHTML)); } catch {}
  fs.writeFileSync(path.join(run,"result.json"),JSON.stringify({outcome:"fail",testedCommit,journey:stage,expected:stage,observed:String(error),classification:"ambiguous",passed},null,2));
  console.error(`FAIL ${stage}: ${error}`); process.exitCode=1;
} finally {
  await crash(); fs.closeSync(log); console.log(`Artifacts: ${run}`);
}

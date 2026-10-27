// D-20261027-OF: real account lookup shim, kernel lease and WebKit app.
// Intentional claim failure is confined to this run's private account home.
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { accountHomeFixture } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { shotOwnerProtection } from "./shot-owner-protection.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";

ensurePrivateSessionBus();
const root=path.resolve(import.meta.dirname,"..");
const artifacts=path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root,"artifacts/sb1-owner/native"));
fs.mkdirSync(artifacts,{recursive:true});
const run=fs.mkdtempSync(path.join(artifacts,"run-"));
const fixture=accountHomeFixture(run,run,{...process.env,TINE_E2E_ACCOUNT_HOME:undefined,TINE_E2E_ACCOUNT_SHIM:undefined,TINE_E2E_EXPECT_OWNER_UNAVAILABLE:"intentional-permissions-failure"});
const owners=path.join(fixture.env.TINE_E2E_ACCOUNT_HOME,".local/state/tine/owners");
fs.mkdirSync(owners,{recursive:true,mode:0o700}); fs.chmodSync(owners,0o755);
const graph=path.join(run,"graph"), file=path.join(graph,"pages/Owner.md");
fs.mkdirSync(path.dirname(file),{recursive:true}); fs.mkdirSync(path.join(graph,"logseq"));
fs.writeFileSync(file,"- editable\n");
fs.writeFileSync(path.join(graph,"logseq/config.edn"),'{:default-home {:page "Owner"}}\n');
const app=path.join(run,"tine"); fs.copyFileSync(process.env.TINE_APP || path.join(root,"target/debug/tine"),app); fs.chmodSync(app,0o700);
const env={...fixture.env,TINE_GRAPH:graph,TINE_OWNER_PROTECTION_RUN:run,TAURI_WEBVIEW_AUTOMATION:"true",
  XDG_DATA_HOME:path.join(run,"data"),XDG_CONFIG_HOME:path.join(run,"config"),XDG_CACHE_HOME:path.join(run,"cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER:"1",WEBKIT_DISABLE_COMPOSITING_MODE:"1",LIBGL_ALWAYS_SOFTWARE:"1",GDK_BACKEND:"x11"};
const testedCommit=execFileSync("rtk",["git","rev-parse","HEAD"],{cwd:root,encoding:"utf8"}).trim();
const port=()=>new Promise((resolve,reject)=>{const server=net.createServer();server.on("error",reject);server.listen(0,"127.0.0.1",()=>{const number=server.address().port;server.close(error=>error?reject(error):resolve(number));});});
const log=fs.openSync(path.join(run,"driver.log"),"w");
let driver,browser,serve,stage="launch editable fallback";
const passed=[];
const notice='[data-testid="owner-protection-notice"]';
const retryOnly=process.env.E2E_OWNER_RETRY_ONLY==="1";
const click=async label=>browser.execute((selector,label)=>[...document.querySelectorAll(`${selector} button`)].find(b=>b.textContent===label)?.click(),notice,label);
const status=()=>new Promise((resolve,reject)=>{
  const child=spawn(app,["status","--graph",graph,"--json"],{env,timeout:10000});let out="",err="";
  child.stdout.on("data",d=>out+=d);child.stderr.on("data",d=>err+=d);child.on("error",reject);
  child.on("exit",code=>{try{assert.equal(code,0,err);resolve(JSON.parse(out));}catch(error){reject(error);}});
});
function stopPids() {
  for(const pid of fs.readdirSync("/proc").filter(s=>/^\d+$/.test(s))) {
    try {
      if(fs.readFileSync(`/proc/${pid}/environ`,"utf8").split("\0").includes(`TINE_OWNER_PROTECTION_RUN=${run}`)
        && fs.readlinkSync(`/proc/${pid}/exe`)===app) process.kill(Number(pid),"SIGTERM");
    } catch {}
  }
}
try {
  const driverPort=await port(), nativePort=await port();
  driver=spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME,"bin/tauri-driver"),["--port",String(driverPort),"--native-port",String(nativePort),"--native-driver",process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],{env,stdio:["ignore",log,log],detached:true});
  await sleep(1500);
  browser=await remote({hostname:"127.0.0.1",port:driverPort,path:"/",logLevel:"error",connectionRetryCount:1,connectionRetryTimeout:30000,
    capabilities:{browserName:"wry","wdio:enforceWebDriverClassic":true,"tauri:options":{application:app}}});
  await ensureMainWindow(browser); await openPageByName(browser,"Owner");
  if (!retryOnly) {
  await browser.$(notice).waitForExist({timeout:10000});
  assert.match(await browser.$(notice).getText(),/Single-writer protection and the command line/);
  assert.equal(fixture.sample().length,0,"intentional failure must not silently hold a lease");
  shotOwnerProtection(path.join(run,"warning.png"),env);
  await browser.waitUntil(()=>browser.execute(()=>{
    const content=document.querySelector(".page .ls-block .block-content-wrapper");
    if(!(content instanceof HTMLElement)) return false;
    content.dispatchEvent(new MouseEvent("mousedown",{bubbles:true,cancelable:true,button:0}));
    content.dispatchEvent(new MouseEvent("mouseup",{bubbles:true,cancelable:true,button:0}));content.click();
    return document.activeElement instanceof HTMLTextAreaElement;
  }),{timeout:10000});
  await browser.keys(["End"]); for(const key of " without lease") await browser.keys([key]);
  await browser.keys(["Escape"]);
  await waitForFileText(file,text=>text==="- editable without lease\n");
  passed.push("claim refusal shows persistent warning while literal typing still saves");

  stage="repair button acquires lease and clears warning";
  await click("Repair permissions");
  await browser.$(notice).waitForExist({reverse:true,timeout:15000});
  assert.equal(fs.statSync(owners).mode & 0o777,0o700);
  assert.equal((await status()).owner,"app");
  assert(fixture.sample().length,"recovery must hold a real kernel lock");
  shotOwnerProtection(path.join(run,"recovered.png"),env);passed.push(stage);

  // A second launch exercises the lifetime after fallback: repair access
  // externally and start serve before the app's next scheduled retry. The app
  // must hand over instead of treating contention as a free lock.
  stage="automatic retry takes over a serve host started after external repair";
  await browser.deleteSession(); browser=undefined;
  try{process.kill(-driver.pid,"SIGTERM");}catch{} driver=undefined;
  stopPids(); await sleep(1000);
  fs.chmodSync(owners,0o755);
  const dp=await port(),np=await port();
  driver=spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME,"bin/tauri-driver"),["--port",String(dp),"--native-port",String(np),"--native-driver",process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],{env,stdio:["ignore",log,log],detached:true});
  await sleep(1500);
  browser=await remote({hostname:"127.0.0.1",port:dp,path:"/",logLevel:"error",connectionRetryCount:1,connectionRetryTimeout:30000,
    capabilities:{browserName:"wry","wdio:enforceWebDriverClassic":true,"tauri:options":{application:app}}});
  await ensureMainWindow(browser);await openPageByName(browser,"Owner");await browser.$(notice).waitForExist({timeout:10000});
  }
  stage="automatic retry takes over a serve host started after external repair";
  // Stop only this fixture's app while arranging a deterministic CLI incumbent.
  // SIGCONT resumes normal production timers; no production test switches.
  const appPid=Number(fs.readdirSync("/proc").find(pid=>{
    try{return /^\d+$/.test(pid) && fs.readlinkSync(`/proc/${pid}/exe`)===app;}catch{return false;}
  }));
  assert(appPid,"fixture app PID");process.kill(appPid,"SIGSTOP");
  try {
    fs.chmodSync(owners,0o700);
    const serveLog=fs.openSync(path.join(run,"serve.log"),"w");
    serve=spawn(app,["serve",graph],{env,stdio:["ignore",serveLog,serveLog]});fs.closeSync(serveLog);
    for(let n=0;;n++) {
      const s=await status();if(s.owner==="serve")break;
      if(n===30)throw new Error("CLI serve never became incumbent");await sleep(100);
    }
    assert.equal((await status()).pid,serve.pid);
  } finally {process.kill(appPid,"SIGCONT");}
  await browser.waitUntil(async()=>{
    try{return (await status()).owner==="app";}catch{return false;}
  },{timeout:20000,timeoutMsg:"automatic retry did not take over CLI serve"});
  await browser.$(notice).waitForExist({reverse:true,timeout:5000});
  const recovered=await status();assert.equal(recovered.owner,"app");assert.equal(recovered.pid,appPid);
  await browser.waitUntil(()=>serve.exitCode!==null,{timeout:10000});
  assert.equal(fs.readFileSync(file,"utf8"),retryOnly?"- editable\n":"- editable without lease\n");passed.push(stage);
  fixture.verify();
  fs.writeFileSync(path.join(run,"result.json"),JSON.stringify({outcome:"pass",testedCommit,passed},null,2));
  console.log(`PASS: ${passed.join("; ")}`);
} catch(error) {
  try{shotOwnerProtection(path.join(run,"failure.png"),env);}catch{}
  try{fs.writeFileSync(path.join(run,"failure-dom.html"),await browser.execute(()=>document.documentElement.outerHTML));}catch{}
  fs.writeFileSync(path.join(run,"result.json"),JSON.stringify({outcome:"fail",testedCommit,journey:stage,expected:stage,observed:String(error),classification:"ambiguous",passed},null,2));
  console.error(`FAIL ${stage}: ${error}`);process.exitCode=1;
} finally {
  try{await browser?.deleteSession();}catch{} try{process.kill(-driver?.pid,"SIGTERM");}catch{}
  serve?.kill("SIGTERM");stopPids();fixture.close();fs.closeSync(log);console.log(`Artifacts: ${run}`);
}

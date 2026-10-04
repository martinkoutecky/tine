// GH #181: one focused Linux native journey, disposable graphs and app data.
import { spawn } from "node:child_process";
import { setTimeout as poll } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
ensurePrivateSessionBus();
if (!process.env.DISPLAY) throw new Error("Run this focused native journey with xvfb-run -a");
const app = process.env.TINE_APP;
if (!app) throw new Error("Set TINE_APP to the lane's debug custom-protocol binary");
const tmp = fs.mkdtempSync("/tmp/tine-links-");
const graph = path.join(tmp, "graph");
const copy = path.join(tmp, "copy");
const graphId = "11111111-1111-4111-8111-111111111111";
const blockId = "22222222-2222-4222-8222-222222222222";
const pageBytes = `- Linked block\n  id:: ${blockId}\n  - Child proof\n- Other block\n`;
for (const root of [graph, copy]) {
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(root, dir), { recursive: true });
  fs.writeFileSync(path.join(root, "pages/Link Page.md"), pageBytes);
  fs.writeFileSync(path.join(root, "pages/Copy Source.md"), "- Assign only on explicit copy\n");
  fs.writeFileSync(path.join(root, "logseq/tine-graph-id"), graphId + "\n");
}
fs.writeFileSync(path.join(copy,"pages/Other Copy.md"),"- Present in the other copy only\n");
const xdg = path.join(tmp, "xdg");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(xdg, dir), { recursive: true });
const settingsDir = path.join(xdg, "data", APP_ID);
fs.mkdirSync(settingsDir, { recursive: true });
fs.writeFileSync(path.join(settingsDir, "tine-settings.json"), JSON.stringify({ known_graphs: [{name:"graph",path:graph},{name:"copy",path:copy}], last_graph_path:graph }));
const desktop = path.join(tmp,"tine.desktop");
fs.writeFileSync(desktop, fs.readFileSync(new URL("../src-tauri/tine.desktop",import.meta.url),"utf8")
  .replaceAll("{{name}}","Tine QE6 fixture").replaceAll("{{exec}}",`"${app}"`).replaceAll("{{icon}}",APP_ID));
const env = { ...process.env, TINE_GRAPH: "", XDG_DATA_HOME:path.join(xdg,"data"), XDG_CONFIG_HOME:path.join(xdg,"config"), XDG_CACHE_HOME:path.join(xdg,"cache"), GDK_BACKEND:"x11", WEBKIT_DISABLE_DMABUF_RENDERER:"1", WEBKIT_DISABLE_COMPOSITING_MODE:"1", LIBGL_ALWAYS_SOFTWARE:"1" };
const driverPort = Number(process.env.E2E_DRIVER_PORT || 4486);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4487);
const log = fs.openSync(path.join(tmp,"driver.log"),"w");
const td = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME,"bin/tauri-driver"), ["--port",String(driverPort),"--native-port",String(nativePort),"--native-driver",process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio:["ignore",log,log], detached:true });
const forwarded = [];
let browser;
const pageUrl = `tine://page/Link%20Page?graph=${graphId}`;
async function forward(url) {
  const child = spawn(app,[url],{env,stdio:"ignore"}); forwarded.push(child);
  await new Promise((resolve,reject) => { child.on("error",reject); child.on("exit",(code) => code === 0 ? resolve() : reject(new Error(`forwarder exited ${code}`))); });
}
async function chooseCopy() {
  await browser.$('[role="dialog"][aria-label="Choose graph for Tine link"]').waitForExist({timeout:20000});
  await browser.$('[role="dialog"]').waitForDisplayed({timeout:5000});
  // Let the native compositor present the chooser before screenshot evidence.
  await poll(300);
  await browser.saveScreenshot(path.join(tmp,"graph-choice.png"));
  assert.equal(await browser.execute(() => {
    const dialog = document.querySelector('[role="dialog"]');
    const rect = dialog.getBoundingClientRect();
    return document.elementFromPoint(rect.x+rect.width/2, rect.y+rect.height/2)?.closest('[role="dialog"]') === dialog;
  }), true, "cold graph chooser must appear above Welcome");
  const button = await browser.$('[role="dialog"] button');
  assert.equal(await button.getText(),graph);
  await button.click();
}
async function clipboard(command, text = null) {
  return browser.executeAsync((cmd,value,done) => globalThis.__TAURI_INTERNALS__.invoke(`plugin:clipboard-manager|${cmd}`, value === null ? {} : {text:value}).then(done, (error) => done({error:String(error)})),command,text);
}
try {
  const deadline = Date.now()+10000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${driverPort}/status`)).ok) break; } catch {}
    if (Date.now()>deadline) throw new Error("tauri-driver did not become ready");
    await poll(100);
  }
  browser = await remote({ hostname:"127.0.0.1",port:driverPort,path:"/",logLevel:"error",connectionRetryCount:0,connectionRetryTimeout:60000,
    capabilities:{browserName:"wry","wdio:enforceWebDriverClassic":true,"tauri:options":{application:app,args:[pageUrl]}} });
  await chooseCopy();
  await browser.waitUntil(() => browser.execute(() => document.querySelector("h1.page-title")?.textContent === "Link Page"), {timeout:20000,timeoutMsg:"cold external URL did not open its page"});
  assert.equal(fs.readFileSync(path.join(graph,"pages/Link Page.md"),"utf8"),pageBytes);
  await browser.saveScreenshot(path.join(tmp,"page-link.png"));
  await forward(`tine://block/${blockId}`);
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".zoomed-block")?.textContent.includes("Child proof")), {timeout:20000,timeoutMsg:"warm UUID-only URL did not zoom to the block"});
  assert.equal(await browser.$('[role="dialog"][aria-label="Choose graph for Tine link"]').isExisting(),false,"remembered copy must not prompt again");
  assert.equal(fs.readFileSync(path.join(graph,"pages/Link Page.md"),"utf8"),pageBytes);
  await forward(`tine://page/Other%20Copy?graph=${graphId}`);
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".toast")].some((toast) => toast.textContent.includes("target not found in the chosen graph copy"))), {timeout:10000,timeoutMsg:"a remembered copy must not fall back to another copy containing the target"});
  assert.ok(await browser.$(".zoomed-block").isExisting());
  assert.equal(fs.existsSync(path.join(graph,"pages/Other Copy.md")),false);
  await forward(`tine://page/Missing?graph=${graphId}`);
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".toast")].some((toast) => toast.textContent.includes("target not found"))), {timeout:10000,timeoutMsg:"missing target lacked an error"});
  assert.equal(fs.existsSync(path.join(graph,"pages/Missing.md")),false);
  assert.ok(await browser.$(".zoomed-block").isExisting(),"missing URL must retain prior destination");
  await openPageByName(browser,"Copy Source");
  await clipboard("write_text","sentinel");
  await browser.execute(() => document.querySelector('[data-page-actions-trigger]')?.click());
  await browser.$('[data-page-action-id="copy-link"]').click();
  await browser.waitUntil(async () => (await clipboard("read_text")) === `tine://page/Copy%20Source?graph=${graphId}`,{timeout:10000,timeoutMsg:"Page Copy link did not publish its external URL"});
  await clipboard("write_text","sentinel");
  await browser.execute(() => {
    const bullet = document.querySelector(".page-blocks .ls-block .bullet-container");
    if (!bullet) throw new Error("copy source bullet missing");
    bullet.dispatchEvent(new MouseEvent("contextmenu",{bubbles:true,cancelable:true,clientX:100,clientY:150}));
  });
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".ctx-item")].some((item) => item.textContent.trim() === "Copy link")),{timeout:5000});
  await browser.execute(() => [...document.querySelectorAll(".ctx-item")].find((item) => item.textContent.trim() === "Copy link").click());
  let copied;
  await browser.waitUntil(async () => { copied = await clipboard("read_text"); return typeof copied === "string" && copied.startsWith("tine://block/"); }, {timeout:10000,timeoutMsg:"Block Copy link did not reach clipboard"});
  const assigned = copied.slice("tine://block/".length);
  assert.ok(fs.readFileSync(path.join(graph,"pages/Copy Source.md"),"utf8").includes(`id:: ${assigned}`),"block ID must be durable before clipboard publication");
  assert.equal(fs.readFileSync(path.join(copy,"pages/Copy Source.md"),"utf8"),"- Assign only on explicit copy\n");
  const shell = spawn("gio",["launch",desktop,pageUrl],{env,stdio:"ignore"});
  await new Promise((resolve,reject) => { shell.on("error",reject); shell.on("exit",(code) => code === 0 ? resolve() : reject(new Error(`desktop launcher exited ${code}`))); });
  await browser.waitUntil(() => browser.execute(() => document.querySelector("h1.page-title")?.textContent === "Link Page"), {timeout:15000,timeoutMsg:"the shipped Linux desktop template must forward a URL"});
  await openPageByName(browser,"Copy Source");
  const main = await browser.getWindowHandle();
  const initial = await browser.getWindowHandles();
  await forward(copy);
  await browser.waitUntil(async () => (await browser.getWindowHandles()).length > initial.length, {timeout:15000});
  const peer = (await browser.getWindowHandles()).find((handle) => !initial.includes(handle));
  await browser.switchToWindow(peer);
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".graph-switch-name")?.textContent.trim() === "copy"), {timeout:15000,timeoutMsg:"a later graph window must load its graph after a cold URL launch"});
  await openPageByName(browser,"Copy Source");
  await forward(pageUrl);
  await browser.switchToWindow(main);
  await browser.waitUntil(() => browser.execute(() => document.querySelector("h1.page-title")?.textContent === "Link Page"), {timeout:15000,timeoutMsg:"a link received by another graph window must reach its existing owner"});
  await browser.switchToWindow(peer);
  assert.equal(await browser.execute(() => document.querySelector("h1.page-title")?.textContent),"Copy Source","handoff must keep the receiving window's route");
  await browser.saveScreenshot(path.join(tmp,"peer-window.png"));
  console.log(`PASS: cold/warm links, remembered graph-copy choice, missing-target refusal, durable menu copy, and graph-window handoff. Evidence: ${tmp}`);
} catch (error) {
  if (browser) { await browser.saveScreenshot(path.join(tmp,"failure.png")).catch(() => {}); fs.writeFileSync(path.join(tmp,"failure.txt"),await browser.execute(() => document.body.innerText).catch(() => "unavailable")); }
  throw error;
} finally {
  await browser?.deleteSession().catch(() => {});
  for (const child of forwarded) if (child.exitCode === null) child.kill();
  try { process.kill(-td.pid,"SIGTERM"); } catch { td.kill(); }
  fs.closeSync(log);
}

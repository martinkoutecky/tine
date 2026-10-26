// CLI §3: serve -> app handover, immediate window update, typing preservation.
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const gitHead = () => { try { return execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(); } catch { return "unknown"; } };
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";

ensurePrivateSessionBus();

const APP = path.resolve(process.env.TINE_APP || "target/debug/tine");
const TMP = path.resolve(process.env.E2E_TMP_DIR || `artifacts/cli-native-${process.pid}`);
assert(!fs.existsSync(TMP), "use a fresh artifact directory");
const GRAPH = path.join(TMP, "graph");
for (const sub of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, sub), { recursive: true });
const file = path.join(GRAPH, "pages/CLI.md");
fs.writeFileSync(file, "- user\n- agent\n");
const typingFile = path.join(GRAPH,"pages/Typing.md");
fs.writeFileSync(typingFile,"- user\n- agent\n");
fs.writeFileSync(path.join(GRAPH, "pages/Referrer.md"), "- see [[CLI]]\n");
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), '{:default-home {:page "CLI"}}\n');
const env = { ...process.env, TAURI_WEBVIEW_AUTOMATION: "true", TINE_GRAPH: GRAPH, TINE_RUN: "", XDG_DATA_HOME: path.join(TMP,"data"), XDG_CONFIG_HOME: path.join(TMP,"config"), XDG_CACHE_HOME: path.join(TMP,"cache"), XDG_STATE_HOME: path.join(TMP,"state"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4710);
const native = Number(process.env.E2E_NATIVE_PORT || 4711);
let driver, browser, serve;
const steps = [];
const failures = [];
function cli(args, expected = 0) {
  return new Promise((resolve, reject) => {
    const child = spawn(APP, [...args, "--graph", GRAPH, "--json"], { env, timeout:60_000 });
    let out = "", err = "";
    child.stdout.on("data", d => out += d);
    child.stderr.on("data", d => err += d);
    child.on("error", reject);
    child.on("exit", code => {
      try {
        const reply = JSON.parse(code ? err : out);
        if (expected === null) resolve({exit:code,reply});
        else { assert.equal(code, expected, `${args[0]}: ${err}`); resolve(reply); }
      }
      catch (e) { reject(e); }
    });
  });
}
const handle = (page, i) => `${page.page.name}/${page.page.blocks[i].handle}`;
try {
  const log = fs.openSync(path.join(TMP,"serve.log"), "w");
  serve = spawn(APP, ["serve", GRAPH], { env, stdio: ["ignore",log,log] });
  fs.closeSync(log);
  await sleep(1000);
  assert.equal((await cli(["status"])).owner, "serve");
  const page = await cli(["page", "show", "CLI"]);
  await cli(["edit", handle(page,1), "agent before handover"]);
  const driverLog = fs.openSync(path.join(TMP,"driver.log"), "w");
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port,native,process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore",driverLog,driverLog], detached:true });
  fs.closeSync(driverLog);
  await sleep(2000);
  console.log("driver ready; creating native session");
  browser = await remote({ hostname:"127.0.0.1",port,path:"/",logLevel:"error",connectionRetryCount:1,connectionRetryTimeout:60_000,capabilities:tauriCapabilities(APP) });
  await ensureMainWindow(browser);
  console.log("native session ready");
  await browser.$(".page-title, .ls-block").waitForExist({timeout:20_000});
  await openPageByName(browser,"CLI");
  assert.equal((await cli(["status"])).owner, "app");
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".page")?.textContent.includes("agent before handover")), {timeout:15_000});
  assert.equal(fs.readFileSync(file,"utf8"), "- user\n- agent before handover\n");
  steps.push("handover: app owns graph and displays durable serve edit");

  const open = await cli(["page","show","CLI"]);
  await browser.execute(() => { window.__cliErrors = []; window.addEventListener("error", e => window.__cliErrors.push(e.message)); window.addEventListener("unhandledrejection", e => window.__cliErrors.push(String(e.reason))); });
  await cli(["edit",handle(open,1),"agent visible immediately"]);
  fs.writeFileSync(path.join(TMP,"after-write-dom.json"),JSON.stringify(await browser.execute(() => ({text:document.body.innerText,errors:window.__cliErrors})),null,2));
  await waitForFileText(file, text => text === "- user\n- agent visible immediately\n");
  try {
    await browser.waitUntil(() => browser.execute(() => document.querySelector(".page")?.textContent.includes("agent visible immediately")), {timeout:15_000});
    steps.push("open window: CLI write appears without navigation");
  } catch (error) {
    failures.push({journey:"open-window",classification:"product",expected:"CLI write appears without navigation",observed:"file changed; window still displays old text"});
    fs.writeFileSync(path.join(TMP,"window-failure-dom.json"),JSON.stringify(await browser.execute(() => ({text:document.body.innerText,errors:window.__cliErrors})),null,2));
  }

  // Independent page: a failed window notification must not hide typing proof.
  await openPageByName(browser,"Typing");
  const beforeTyping = await cli(["page","show","Typing"]);
  await browser.waitUntil(() => browser.execute(() => {
    const content = document.querySelector(".page .ls-block .block-content-wrapper");
    if (!(content instanceof HTMLElement)) return false;
    content.dispatchEvent(new MouseEvent("mousedown",{bubbles:true,cancelable:true,button:0}));
    content.dispatchEvent(new MouseEvent("mouseup",{bubbles:true,cancelable:true,button:0}));
    content.click();
    return document.activeElement instanceof HTMLTextAreaElement;
  }), {timeout:10_000,interval:200});
  await browser.keys(["End"]);
  for (const key of " typing survives") await browser.keys([key]);
  const typed = await browser.execute(() => document.activeElement instanceof HTMLTextAreaElement ? document.activeElement.value : null);
  assert.equal(typed,"user typing survives","literal key events must reach the intended editor before invoking CLI");
  fs.writeFileSync(path.join(TMP,"typing-before.json"),JSON.stringify({editor:typed,file:fs.readFileSync(typingFile,"utf8")},null,2));
  // Still in the editor: flush must save the input before resolving the old handle.
  const result = await cli(["edit",handle(beforeTyping,0),"agent must not overwrite user"],null);
  if (result.exit !== 6 || result.reply.error?.code !== "stale") {
    failures.push({journey:"typing",classification:"product",expected:"flush user input, then refuse obsolete handle as stale",observed:`CLI exit ${result.exit} while editor contains changed text`});
    await sleep(2000);
    fs.writeFileSync(path.join(TMP,"typing-result.json"),JSON.stringify({result,file:fs.readFileSync(typingFile,"utf8"),editors:await browser.execute(() => [...document.querySelectorAll("textarea")].map(e=>e.value))},null,2));
  } else {
    await waitForFileText(typingFile, text => text === "- user typing survives\n- agent\n");
    assert(await browser.execute(() => document.activeElement instanceof HTMLTextAreaElement && document.activeElement.value.includes("typing survives")));
    // An unrelated block can still be edited while the user keeps that editor open.
    await cli(["edit",handle(beforeTyping,1),"agent alongside typing"]);
    await waitForFileText(typingFile, text => text === "- user typing survives\n- agent alongside typing\n");
    steps.push("typing: stale refusal preserves user input; unrelated write succeeds");
  }
  await openPageByName(browser,"CLI");
  await cli(["page","rename","CLI","CLI renamed"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".page-title")?.textContent.includes("CLI renamed")), {timeout:15_000});
  fs.writeFileSync(path.join(TMP,"rename-source-dom.json"), JSON.stringify(await browser.execute(() => ({ title:document.querySelector(".page-title")?.textContent, text:document.querySelector(".page")?.textContent })),null,2));
  assert(!fs.existsSync(file));
  assert(fs.existsSync(path.join(GRAPH,"pages/CLI renamed.md")));
  await openPageByName(browser,"Referrer");
  await cli(["page","rename","CLI renamed","CLI final"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".page")?.textContent.includes("CLI final")), {timeout:15_000});
  fs.writeFileSync(path.join(TMP,"rename-referrer-dom.json"), JSON.stringify(await browser.execute(() => ({ title:document.querySelector(".page-title")?.textContent, text:document.querySelector(".page")?.textContent })),null,2));
  assert.equal(fs.readFileSync(path.join(GRAPH,"pages/Referrer.md"),"utf8"),"- see [[CLI final]]\n");
  steps.push("page rename: open source navigation and open referrer text follow host publication");
  await openPageByName(browser,"CLI final");
  fs.mkdirSync(path.join(GRAPH,"logseq/.tine-trash/pages"),{recursive:true});
  fs.chmodSync(path.join(GRAPH,"logseq"),0o555);
  let partial;
  try { partial = await cli(["page","rename","CLI final","CLI partial"],8); }
  finally { fs.chmodSync(path.join(GRAPH,"logseq"),0o755); }
  assert.equal(partial.error.code,"uncertain");
  assert.equal(partial.error.detail.admitted,true);
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".page-title")?.textContent.includes("CLI partial")), {timeout:15_000});
  assert.equal(fs.readFileSync(path.join(GRAPH,"pages/Referrer.md"),"utf8"),"- see [[CLI partial]]\n");
  assert(fs.readFileSync(path.join(GRAPH,"logseq/config.edn"),"utf8").includes('"CLI final"'));
  fs.writeFileSync(path.join(TMP,"rename-partial-dom.json"), JSON.stringify(await browser.execute(() => ({ title:document.querySelector(".page-title")?.textContent, text:document.querySelector(".page")?.textContent })),null,2));
  steps.push("partial rename: home config failure returns uncertain and the open source route follows known publication");
  fs.writeFileSync(path.join(TMP,"steps.json"),JSON.stringify(steps,null,2)+"\n");
  if (failures.length) throw new Error("native journey failure: " + failures.map(f => f.journey).join(", "));
  console.log(`PASS ${steps.join("; ")}`);
} catch (error) {
    try { fs.writeFileSync(path.join(TMP,"failure-dom.json"),JSON.stringify(await browser.execute(() => ({text:document.body.innerText,editors:[...document.querySelectorAll("textarea")].map(e=>e.value),errors:window.__cliErrors})),null,2)); } catch {}
  try { await browser?.saveScreenshot(path.join(TMP,"failure.png")); } catch {}
  fs.writeFileSync(path.join(TMP,"failure.json"),JSON.stringify({commit:gitHead(),journey:steps.length,expected:"handover, window update and typing preservation",observed:String(error),classification:failures.length ? "product" : "ambiguous",steps,failures},null,2));
  console.error(error);
  process.exitCode=1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid,"SIGKILL"); } catch {}
  serve?.kill("SIGTERM");
  // Only processes launched against this journey's disposable graph.
  for (const pid of fs.readdirSync("/proc").filter(p => /^\d+$/.test(p))) {
    try {
      if (fs.readFileSync(`/proc/${pid}/environ`,"utf8").split("\0").includes(`TINE_GRAPH=${GRAPH}`)
        && path.basename(fs.readlinkSync(`/proc/${pid}/exe`)) === path.basename(APP)) process.kill(Number(pid),"SIGKILL");
    } catch {}
  }
}

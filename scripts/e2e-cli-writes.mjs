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
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";

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
// #840: an edit keeps the properties its text does not name, under either owner.
const propsFile = path.join(GRAPH,"pages/Props.md");
fs.writeFileSync(propsFile,"- serve\n  status:: done\n- app\n  status:: done\n");
// #842: parallel appends to one page against the app, with it open and closed.
const parallelFiles = { "Parallel Open": path.join(GRAPH,"pages/Parallel Open.md"), "Parallel Closed": path.join(GRAPH,"pages/Parallel Closed.md") };
for (const f of Object.values(parallelFiles)) fs.writeFileSync(f,"- existing one\n- existing two\n");
fs.writeFileSync(path.join(GRAPH, "pages/Referrer.md"), "- see [[CLI]]\n");
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), '{:default-home {:page "CLI"}}\n');
const ownerFixture = installAccountHome(TMP, process.env.E2E_ARTIFACT_DIR || TMP);
const env = { ...process.env, TAURI_WEBVIEW_AUTOMATION: "true", TINE_GRAPH: GRAPH, TINE_RUN: "", XDG_DATA_HOME: path.join(TMP,"data"), XDG_CONFIG_HOME: path.join(TMP,"config"), XDG_CACHE_HOME: path.join(TMP,"cache"), XDG_STATE_HOME: path.join(TMP,"state"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4710);
const native = Number(process.env.E2E_NATIVE_PORT || 4711);
let driver, browser, serve;
const steps = [];
const failures = [];
function cli(args, expected = 0, overrides = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(APP, [...args, "--graph", GRAPH, "--json"], { env: { ...env, ...overrides }, timeout:60_000 });
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
  // #930: the CLI's environment may differ from the already-running owner's.
  for (const author of ["", " \t"]) {
    const before = fs.readFileSync(file, "utf8");
    await cli(["add", "--page", "CLI", "- refused", "--author", author], 6);
    assert.equal(fs.readFileSync(file, "utf8"), before);
  }
  const authorFile = path.join(GRAPH, "pages/author.md");
  for (const author of ["", " \t", "  Shell Author  "]) {
    await cli(["add", "--page", "author", "- attributed"], 0, { TINE_AUTHOR: author });
  }
  assert.equal(fs.readFileSync(authorFile, "utf8"), "- attributed\n  tine.author:: agent\n- attributed\n  tine.author:: agent\n- attributed\n  tine.author:: Shell Author\n");
  steps.push("blank author refuses without writing; client environment trims and defaults on add");
  const page = await cli(["page", "show", "CLI"]);
  await cli(["edit", handle(page,1), "agent before handover"]);
  await cli(["edit", handle(await cli(["page","show","Props"]),0), "serve edited"]);
  await waitForFileText(propsFile, text => text.startsWith("- serve edited\n  status:: done\n"));
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
  await assertAppLease(ownerFixture);
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

  await cli(["edit",handle(await cli(["page","show","Props"]),1),"app edited"]);
  await waitForFileText(propsFile, text => text === "- serve edited\n  status:: done\n- app edited\n  status:: done\n");
  steps.push("both owners: edit keeps the block's properties");

  // #842: the app adds a window flush and reload per write; all must still land.
  const parallelAdds = async (name, round) => {
    const results = await Promise.all([1,2,3,4,5,6].map(i => cli(["add","--page",name,`- ${name} ${round}-${i}`],null)));
    const failed = results.filter(r => r.exit !== 0);
    assert.deepEqual(failed, [], `${name} round ${round}: every parallel add lands`);
    const text = fs.readFileSync(parallelFiles[name],"utf8");
    assert(text.startsWith("- existing one\n- existing two\n"), "existing blocks stay first and in order");
    for (const i of [1,2,3,4,5,6]) assert.equal(text.split(`- ${name} ${round}-${i}\n`).length, 2, `${name} ${round}-${i} once`);
  };
  await openPageByName(browser,"Parallel Open");
  for (const round of [1,2,3]) await parallelAdds("Parallel Open",round);
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".page")?.textContent.includes("Parallel Open 3-6") && document.querySelector(".page")?.textContent.includes("Parallel Open 3-1")), {timeout:15_000});
  for (const round of [1,2,3]) await parallelAdds("Parallel Closed",round);
  steps.push("parallel adds: all land with the page open in the window and with it closed");

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

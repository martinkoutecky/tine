// GH #800: a routed page holding BOTH a refused save (its file changed on disk
// under unsaved input) and a sync tool's conflict copy is resolvable: the save
// conflict's review comes first, then the copy's, and every version survives.
// Synthetic scratch graph and app data only. Run a custom-protocol debug build
// under xvfb-run (TINE_APP overrides target/debug/tine).
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "target/artifacts/gh800");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const original = "- alpha\n- beta\n- gamma\n";
const copyText = "- alpha\n- beta COPY\n- gamma\n";
const file = path.join(graph, "pages/Lock Demo.md");
const copy = path.join(graph, "pages/Lock Demo.sync-conflict-20261028-222343-ABCDEFG.md");
fs.writeFileSync(file, original);
fs.writeFileSync(copy, copyText);
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4896);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(port, port + 1, "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex") };
let browser, stage = "startup";
const shot = name => {
  try { execFileSync("import", ["-window", "root", path.join(out, name)], { env, timeout: 10000 }); } catch {}
};
const SAVE_FIRST = "Resolve this page’s save conflict first";
const panelSource = () => browser.execute(() => document.querySelector(".main-content .page-conflict")?.getAttribute("data-source") ?? null);
const clickPanel = async prefix => assert.equal(await browser.execute(prefix => {
  const button = [...document.querySelectorAll(".main-content .page-conflict button")]
    .find(el => el.textContent.trim().startsWith(prefix));
  button?.click();
  return Boolean(button);
}, prefix), true, `missing conflict action ${prefix}`);
const toastTexts = () => browser.execute(() => [...document.querySelectorAll(".toast")].map(el => el.textContent));
try {
  await sleep(1200);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 0, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app, "gh800-save-then-copy") });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Lock Demo");
  await browser.waitUntil(async () => (await panelSource()) === "sync-copy", { timeout: 20000,
    timeoutMsg: "the sync copy's panel never appeared" });

  // The reporter's state: unsaved input on the page while another writer
  // changes its file. The write races the 400 ms send debounce; a lost race
  // (the edit saved first) only reloads the page, so try again.
  stage = "unsaved input meets a file changed on disk";
  let conflicted = false;
  for (let attempt = 1; attempt <= 6 && !conflicted; attempt++) {
    assert.ok(await browser.execute(() => {
      for (const node of document.querySelectorAll("[data-gh800-target]")) node.removeAttribute("data-gh800-target");
      const el = [...document.querySelectorAll(".main-content .ls-block .block-content-wrapper")]
        .find(node => node.textContent.trim().startsWith("alpha"));
      el?.setAttribute("data-gh800-target", "1");
      el?.scrollIntoView({ block: "center" });
      return Boolean(el);
    }), "the alpha block is missing");
    await browser.$("[data-gh800-target]").click();
    await browser.$(".main-content textarea.block-editor").waitForExist({ timeout: 5000 });
    await browser.keys(["End"]);
    await browser.keys(" MINE");
    fs.writeFileSync(file, `- alpha\n- beta\n- gamma DISK${attempt}\n`);
    conflicted = await browser.waitUntil(() => browser.execute(() =>
      [...document.querySelectorAll(".conflict-banner")].some(el => el.textContent.includes("changed on disk while you were editing"))),
    { timeout: 6000 }).then(() => true, () => false);
    if (!conflicted) {
      await browser.keys("Escape");
      await sleep(1500);
    }
  }
  assert.ok(conflicted, "no save conflict could be produced");
  await browser.keys("Escape");
  shot("both-conflicts.png");

  stage = "the save conflict's review is shown first and names the copy as next";
  await browser.waitUntil(async () => (await panelSource()) === "live-save", { timeout: 15000,
    timeoutMsg: "the page did not offer the save conflict's review (GH #800: the sync-copy panel hid it)" });
  assert.ok(await browser.execute(() => document.querySelector(".main-content .page-conflict")?.textContent
    .includes("Another version of this page also arrived")), "the live review does not say the copy comes next");
  const diskBefore = fs.readFileSync(file, "utf8");
  const gamma = diskBefore.match(/gamma DISK\d+/)[0];
  await clickPanel("Keep both everywhere");
  await clickPanel("Apply resolution");
  const merged = await waitForFileText(file, text => text.includes("MINE") && text.includes(gamma), { timeoutMs: 20000 });
  assert.ok(!(await toastTexts()).some(text => text.includes(SAVE_FIRST)));
  // The page shows what was written: the merge, not the file as it was before
  // the review, and it still renders.
  const shown = () => browser.execute(() => [...document.querySelectorAll(".main-content .page-blocks .ls-block .block-content-wrapper")]
    .map(el => el.textContent.trim()));
  const lines = text => text.split("\n").filter(Boolean).map(line => line.replace(/^- /, ""));
  await browser.waitUntil(async () => JSON.stringify(await shown()) === JSON.stringify(lines(merged)), { timeout: 10000,
    timeoutMsg: `the page does not show the resolution it wrote: ${JSON.stringify(await shown())} vs ${JSON.stringify(lines(merged))}` });
  assert.ok(!(await browser.execute(() => document.body.textContent.includes("could not be displayed"))), "the page crashed");
  shot("live-resolved.png");

  stage = "then the copy's comparison applies through the guarded fold";
  await browser.waitUntil(async () => (await panelSource()) === "sync-copy", { timeout: 15000,
    timeoutMsg: "the copy's panel did not follow the resolved save conflict" });
  await sleep(500);
  await clickPanel("Keep both everywhere");
  await clickPanel("Apply resolution");
  await browser.waitUntil(() => !fs.existsSync(copy), { timeout: 20000,
    timeoutMsg: `the copy was not folded; toasts: ${JSON.stringify(await toastTexts())}` });
  const resolved = await waitForFileText(file, text => text.includes("beta COPY"), { timeoutMs: 10000 });
  assert.ok(resolved.includes("MINE") && resolved.includes(gamma), `a version was lost: ${JSON.stringify(resolved)}`);
  assert.ok(!(await toastTexts()).some(text => text.includes(SAVE_FIRST)), "Apply refused with the save-conflict toast");
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".main-content .page-conflict")), { timeout: 10000 });
  await browser.waitUntil(async () => (await shown()).some(text => text.includes("beta COPY")), { timeout: 10000,
    timeoutMsg: "the page does not show the folded copy" });
  assert.ok(!(await browser.execute(() => document.body.textContent.includes("could not be displayed"))), "the page crashed");
  const recoverable = (dir, want) => fs.existsSync(dir) && fs.readdirSync(dir, { withFileTypes: true }).some(entry => {
    const p = path.join(dir, entry.name);
    return entry.isDirectory() ? recoverable(p, want) : fs.readFileSync(p, "utf8") === want;
  });
  assert.ok(recoverable(path.join(graph, "logseq/.tine-trash"), copyText), "the folded copy is not recoverable");
  // Stable (0.8) has no recovery copy of the reviewed disk bytes on a live
  // resolution: that guarantee is L9-04 (cdb189512, development only), not
  // part of GH #800. Here the disk version survives inside the merge itself
  // ("Keep both everywhere"): every line of it is still in the file.
  for (const line of lines(diskBefore)) assert.ok(lines(resolved).includes(line), `the disk line ${JSON.stringify(line)} was lost`);
  shot("resolved.png");
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify({ ...proof, outcome: "PASS", merged, resolved }, null, 2));
  console.log(`PASS GH800-SAVE-THEN-COPY: ${out}`);
} catch (error) {
  shot("failure.png");
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Save conflict reviewed first, then the sync copy folded; the draft, the disk change and the copy all kept",
    observed: String(error.stack), classification: "ambiguous; inspect driver log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}

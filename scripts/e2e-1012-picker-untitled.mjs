// GH #1012: a date picked on a block without title text is written after an
// empty first line (`-` then `  SCHEDULED: <d>`), which Logseq reads as a date.
// On a routed page: create an empty block, pick Scheduled (slash menu by
// keyboard, then by mouse), wait for the file, type a task title, leave the editor,
// wait for the file again, and find the block in today's journal agenda.
// Run once per editor (E2E_EDITOR=textarea or cm6; default cm6):
// source scripts/env.sh; npx tauri build --no-bundle
// E2E_EDITOR=textarea xvfb-run -a node scripts/e2e-1012-picker-untitled.mjs
// E2E_EDITOR=cm6 xvfb-run -a node scripts/e2e-1012-picker-untitled.mjs
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openJournals, openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { editorSelector, editorText } from "./lib/e2e-block-editor.mjs";
import { seedClassicEditorProfile } from "./lib/e2e-classic-profile.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/release/tine");
const kind = process.env.E2E_EDITOR === "textarea" ? "textarea" : "cm6";
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/1012-picker-untitled"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, `native-${kind}-`));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const file = path.join(graph, "pages/Plan.md");
fs.writeFileSync(file, "- seed\n");
const pad = n => String(n).padStart(2, "0");
const now = new Date();
const today = `<${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${now.toLocaleString("en-US", { weekday: "short" })}>`;
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
if (kind === "textarea") seedClassicEditorProfile({ ...env, E2E_CLASSIC_PROFILE: "1" });
const port = Number(process.env.E2E_DRIVER_PORT || 4978);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME ?? "", "bin/tauri-driver"),
  webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  workingDiff: true, app, editor: kind, binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "Pick Scheduled on an empty block of a routed page, type a title, leave the editor, open the journal",
  expected: "The file holds `-` then `  SCHEDULED: <today>`, then `- TODO x` then the same line; today's agenda lists it", steps: [] };
let browser;
let stage = "startup";
const editor = () => browser.$(editorSelector(".main-content"));
const value = () => editorText(browser, ".main-content");

async function emptyBlockAfterSeed(title) {
  await openPageByName(browser, "Plan");
  const rows = await browser.$$(".main-content .block-content-wrapper");
  await rows[rows.length - 1].click();
  await editor().waitForExist({ timeout: 10000 });
  await browser.keys(["End"]);
  await browser.keys(["Enter"]);
  await browser.waitUntil(async () => (await value()) === "", { timeout: 10000 });
  return title;
}
async function pickToday(byMouse) {
  for (const ch of byMouse ? "/sched" : "/scheduled") await browser.keys(ch);
  const item = await browser.$("//*[contains(@class,'ac-item')][.//*[contains(@class,'ac-label') and normalize-space()='Scheduled']]");
  await item.waitForExist({ timeout: 10000 });
  if (byMouse) await item.click(); else await browser.keys(["Enter"]);
  await browser.$(".date-picker").waitForExist({ timeout: 10000 });
  // The picker opens on today; Enter applies the day under the cursor.
  await browser.waitUntil(() => browser.execute(() => !!document.activeElement?.closest(".date-picker")), { timeout: 10000 });
  await browser.keys(["Enter"]);
  await browser.$(".date-picker").waitForExist({ reverse: true, timeout: 10000 });
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2,
    connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, `1012-picker-untitled-${kind}`) });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  let expected = "- seed\n";
  // The agenda lists open items, so each title is a task.
  for (const [title, byMouse] of [["TODO x", false], ["TODO y", true]]) {
    stage = `${title}: empty block`;
    await emptyBlockAfterSeed(title);
    stage = `${title}: pick (${byMouse ? "mouse" : "keyboard"})`;
    await pickToday(byMouse);
    const dated = `${expected}-\n  SCHEDULED: ${today}\n`;
    await waitForFileText(file, text => text === dated);
    proof.steps.push(`${title}: picker wrote \`-\` then the date line`);
    stage = `${title}: type the title`;
    await browser.keys(title);
    await browser.keys(["Escape"]);
    expected = `${expected}- ${title}\n  SCHEDULED: ${today}\n`;
    await waitForFileText(file, text => text === expected);
    proof.steps.push(`${title}: the typed character went into the title line`);
  }
  stage = "agenda";
  await openJournals(browser);
  await browser.waitUntil(() => browser.execute(() => {
    const rows = [...document.querySelectorAll(".agenda-block .ls-block")].map(row => row.textContent ?? "");
    return ["x", "y"].every(t => rows.some(r => r.includes(t)));
  }), { timeout: 30000 });
  proof.steps.push("today's agenda lists both blocks");
  proof.outcome = "PASS";
  console.log(`PASS 1012 picker on an untitled block (${kind}): ${out}`);
} catch (error) {
  proof.outcome = "FAIL";
  proof.failure = { stage, observed: String(error.stack), classification: "ambiguous; inspect driver log and DOM",
    file: fs.readFileSync(file, "utf8") };
  if (browser) {
    try { await browser.saveScreenshot(path.join(out, "failure.png")); } catch {}
    try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  }
  console.error(JSON.stringify(proof.failure));
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}

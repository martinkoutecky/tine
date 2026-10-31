// Local Linux run (disposable graph/profile; no deploy):
// source scripts/env.sh; npx tauri build --no-bundle
// xvfb-run -a node scripts/e2e-textarea-numbered-fence.mjs
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/release/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/textarea-numbered-fence"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const cases = [{ name: "Code Prefix", lang: "js" }, { name: "Calc Prefix", lang: "calc" }].map(c => ({
  ...c, file: path.join(graph, "pages", `${c.name}.md`),
  original: `- \`\`\`${c.lang}\n  x\n  \`\`\`\n- untouched\n`,
}));
for (const c of cases) fs.writeFileSync(c.file, c.original);
fs.writeFileSync(path.join(graph, "pages/Other.md"), "- Other page\n");
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4976);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME ?? "", "bin/tauri-driver"),
  webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  workingDiff: true, app, binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "Type 1. space in routed code and calculator bodies, save and reopen",
  expected: "The typed prefix stays literal; both saved fences and unrelated text survive route reopen", steps: [] };
let browser;
let stage = "startup";
const editorValue = () => browser.execute(() => document.querySelector(".main-content textarea.block-editor")?.value);
async function openEditor() {
  await browser.$(".main-content .block-content-wrapper").click();
  await browser.$(".main-content textarea.block-editor").waitForExist({ timeout: 10000 });
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2,
    connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, "textarea-numbered-fence") });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  for (const c of cases) {
    stage = `route and edit ${c.name}`;
    await openPageByName(browser, c.name);
    await openEditor();
    if (await editorValue() !== "x") throw new Error(`${c.name}: expected the projected body x`);
    await browser.keys(["Control", "a"]);
    await browser.keys("Backspace");
    // Native keystrokes, including the final space that used to clear the fence.
    for (const ch of ["1", ".", " "]) await browser.keys(ch);
    stage = `literal prefix in ${c.name}`;
    if (await editorValue() !== "1. ") throw new Error(`${c.name}: typed prefix became ${JSON.stringify(await editorValue())}`);
    // Finish the line so trailing-space trimming at the save boundary is irrelevant.
    await browser.keys("literal");
    await browser.keys("Escape");
    stage = `save ${c.name}`;
    const expected = c.original.replace("  x\n", "  1. literal\n");
    await waitForFileText(c.file, text => text === expected);
    await openPageByName(browser, "Other");
    await openPageByName(browser, c.name);
    stage = `reopen ${c.name}`;
    await openEditor();
    if (await editorValue() !== "1. literal") throw new Error(`${c.name}: reopened body or wrapper differs`);
    await browser.keys("Escape");
    if (fs.readFileSync(c.file, "utf8") !== expected) throw new Error(`${c.name}: route reopen changed saved bytes`);
    proof.steps.push(`${c.name}: native prefix stayed literal, fenced file saved exactly, body reopened`);
  }
  proof.outcome = "PASS";
  console.log(`PASS textarea numbered fence: ${out}`);
} catch (error) {
  proof.outcome = "FAIL";
  proof.failure = { stage, observed: String(error.stack), classification: "ambiguous; inspect driver log and DOM",
    files: cases.map(c => ({ name: c.name, text: fs.readFileSync(c.file, "utf8") })) };
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

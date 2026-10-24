// D-20261023-VT3: real editor projection -> ordinary guarded save -> disk.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

ensurePrivateSessionBus();
const ROOT = path.resolve(import.meta.dirname, "..");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/og-prop-followups/native"));
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "run-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:block-hidden-properties #{:secret}}\n");
const fixtures = [
  { name: "Hidden Markdown", file: "Hidden Markdown.md", text: "- Original body\n  heading:: true  \n  card-last-score:: 5\n  secret:: opaque  \n  owner:: Ada\n", hidden: ["  heading:: true  ", "  card-last-score:: 5", "  secret:: opaque  "], visible: "Original body\nowner:: Ada" },
  { name: "Hidden Org", file: "Hidden Org.org", text: "* Original body\n:PROPERTIES:\n:heading: true  \n:card-last-score: 5\n:secret: opaque  \n:owner: Ada\n:END:\n", hidden: [":heading: true  ", ":card-last-score: 5", ":secret: opaque  "], visible: "Original body\n:PROPERTIES:\n:owner: Ada\n:END:" },
  { name: "Hidden First Root", file: "Hidden First Root.md", text: "- Original body\n  card-repeats:: 2\n  secret:: opaque  \n", hidden: ["  card-repeats:: 2", "  secret:: opaque  "], visible: "Original body", clear: true },
  { name: "Hidden Append Property", file: "Hidden Append Property.md", text: "- Original body\n  id:: 6512bd43-d9ca-4c95-b0b4-1c2d3e4f5a6b\n  body\n", hidden: ["  id:: 6512bd43-d9ca-4c95-b0b4-1c2d3e4f5a6b"], visible: "Original body\nbody", append: true },
];
if (process.env.E2E_CRLF === "1") for (const fixture of fixtures) {
  fixture.text = fixture.text.replaceAll("\n", "\r\n");
  fixture.hidden = fixture.hidden.map(line => `${line}\r`);
}
for (const f of fixtures) fs.writeFileSync(path.join(graph, "pages", f.file), f.text);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4774);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1),
  "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let stage = "startup";
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || path.join(ROOT, "target/debug/tine") } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  for (const f of fixtures) {
    stage = `edit ${f.name}`;
    await openPageByName(browser, f.name);
    const body = await browser.$("//div[contains(@class, 'block-content') and contains(., 'Original body')]");
    await body.waitForDisplayed({ timeout: 15_000 });
    await body.click();
    await browser.$("textarea.block-editor").waitForExist({ timeout: 10_000 });
    const before = await browser.$("textarea.block-editor").getValue();
    // Org's CRLF reader may retain a terminal blank line. The contract is the
    // visible body/user property and absence of metadata, not that blank line.
    if (before.trimEnd() !== f.visible) throw new Error(`wrong visible projection: ${JSON.stringify(before)}`);
    // Interior text edit leaves the visible user key and its Org wrapper alone.
    await browser.keys(["Control", "Home"]);
    await browser.keys(["Control"]);
    for (const char of "Revised ") await browser.keys(char);
    await browser.keys(["Escape"]);
    const file = path.join(graph, "pages", f.file);
    const saved = await waitForFileText(file, text => text.includes("Revised Original body"));
    for (const line of f.hidden) if (!saved.split("\n").includes(line)) throw new Error(`hidden bytes changed: ${JSON.stringify(line)}\n${saved}`);
    if (!f.clear && !f.append && !saved.includes(f.file.endsWith("org") ? ":owner: Ada" : "owner:: Ada")) throw new Error("visible user property lost");
    console.log(`PASS ${f.name}: editor projection and byte-exact hidden lines on disk`);
    if (!f.clear && !saved.includes(f.text.replace("Original body", "Revised Original body"))) throw new Error("text edit moved property rows");
    if (f.append) {
      stage = "append property after body with hidden id";
      await browser.$("//div[contains(@class, 'block-content') and contains(., 'Revised Original body')]").click();
      await browser.$("textarea.block-editor").waitForExist({ timeout: 10_000 });
      await browser.keys(["Control", "End"]);
      await browser.keys(["Control"]);
      await browser.keys(["Shift", "Enter"]);
      await browser.keys(["Shift"]);
      for (const char of "owner:: Ada") await browser.keys(char);
      await browser.keys(["Escape"]);
      const appended = await waitForFileText(file, text => text.includes("owner:: Ada"));
      // Adjacent rows form the one group OG reads; checking exact placement
      // elsewhere would forbid valid joins that preserve the same properties.
      const normalized = appended.replaceAll("\r\n", "\n");
      const id = f.hidden[0].trim();
      if (!normalized.includes(`  owner:: Ada\n  ${id}`) && !normalized.includes(`  ${id}\n  owner:: Ada`)) throw new Error("appended property and retained id are not in one group");
      console.log("PASS append property: OG reads both owner and retained id after guarded save");
    }
    if (f.clear) {
      stage = "clear first-root flashcard";
      await browser.$("//div[contains(@class, 'block-content') and contains(., 'Revised Original body')]").click();
      await browser.$("textarea.block-editor").waitForExist({ timeout: 10_000 });
      await browser.keys(["Control", "a"]);
      await browser.keys(["Control"]);
      await browser.keys(["Backspace"]);
      await browser.keys(["Escape"]);
      const cleared = await waitForFileText(file, text => !text.includes("Original body"));
      if (!cleared.startsWith("-")) throw new Error("cleared block became page properties");
      for (const line of f.hidden) if (!cleared.split("\n").includes(line)) throw new Error("clearing lost hidden rows");
      console.log("PASS first-root clearing: hidden metadata remains on an outline block on disk");
    }
  }
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify({ base: "b90474792 plus round-2 working diff", outcome: "PASS", journeys: fixtures.map(f => f.name) }, null, 2));
  console.log(`PASS hidden metadata survives Markdown and Org editing: ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: b90474792 plus round-2 working diff\nJourney: editor hidden properties, ${stage}\nExpected: metadata absent from textarea, user key visible, metadata lines byte-identical and in order after save; cleared first root remains a block; appended property shares OG's group with retained id.\nObserved: ${error.stack}\nClassification: ambiguous; inspect screenshot/driver.log.\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}

// GH #689: a page or journal whose file holds no body (0 bytes, or only page
// properties) shows one empty bullet that must take the caret and keep it, so
// typed text reaches the file. The text is entered through WebDriver keys into
// whatever has focus: if the caret vanished, nothing persists.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { openJournals, openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME ?? "", "bin/tauri-driver");
const ARTIFACT = path.join(ROOT, "artifacts/gh689/native");
fs.mkdirSync(ARTIFACT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
const now = new Date();
const pad = (n) => String(n).padStart(2, "0");
const TODAY_FILE = path.join(GRAPH, "journals", `${now.getFullYear()}_${pad(now.getMonth() + 1)}_${pad(now.getDate())}.md`);
const file = (name) => path.join(GRAPH, "pages", `${name}.md`);
fs.writeFileSync(TODAY_FILE, "");
fs.writeFileSync(file("Zero bytes"), "");
fs.writeFileSync(file("Emptied here"), "- a word\n");
fs.writeFileSync(file("Properties only"), "tags:: alpha\n");
fs.writeFileSync(path.join(GRAPH, "pages", "Org empty.org"), "");
fs.writeFileSync(file("Sidebar zero"), "");
fs.writeFileSync(file("Clicked only"), "");

const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4891);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim(), binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), journey: "empty page edit (GH #689)", expected: "an empty bullet on a body-less page keeps the caret and typing persists", steps: [] };

/** Click the last body bullet of `scope` (a CSS selector for one page surface)
 *  the way a user does, then type: the text reaches the file only if the
 *  editor that opened is still focused. */
async function clickLastBulletAndType(scope, text) {
  const wrapper = await browser.execute((sel) => {
    const rows = [...document.querySelectorAll(`${sel} [data-block-id] .block-content-wrapper`)];
    const last = rows[rows.length - 1];
    if (!last) return null;
    last.scrollIntoView({ block: "center" });
    return last.closest("[data-block-id]").getAttribute("data-block-id");
  }, scope);
  if (!wrapper) throw new Error(`no body bullet rendered in ${scope}`);
  await browser.$(`${scope} [data-block-id="${wrapper}"] .block-content-wrapper`).click();
  await browser.$(`${scope} textarea.block-editor`).waitForExist({ timeout: 10_000 });
  // The reported failure: the caret appears, then vanishes. Typing into the
  // focused element is the user's outcome either way.
  await browser.keys(text);
  const typed = await browser.execute(() => {
    const active = document.activeElement;
    return active instanceof HTMLTextAreaElement ? active.value : `<focus on ${active?.tagName ?? "nothing"}>`;
  });
  await browser.keys("Escape");
  return typed;
}

async function expectPersisted(target, text, label, extra = () => true) {
  try {
    await waitForFileText(target, (body) => body.includes(text) && extra(body));
  } catch (error) {
    throw new Error(`${label}: ${error.message}`);
  }
  proof.steps.push(label);
}

try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await browser.execute(() => { for (const close of document.querySelectorAll(".toast-sticky .toast-close")) close.click(); });

  // Every case runs, so one report shows each surface's outcome.
  const failures = [];
  const scenario = async (label, run) => {
    try { await run(); } catch (error) {
      failures.push(`${label}: ${String(error).split("\n")[0]}`);
      try { await browser.saveScreenshot(path.join(TMP, `failure-${failures.length}.png`)); } catch {}
      await browser.keys("Escape").catch(() => {});
    }
  };

  // 1. A routed page emptied in Tine (the reporter's path): its file loses its
  //    body, and the bullet left behind must still take typing.
  await scenario("emptied in Tine", async () => {
    await openPageByName(browser, "Emptied here");
    await browser.$(".main-content [data-block-id] .block-content-wrapper").click();
    await browser.$(".main-content textarea.block-editor").waitForExist({ timeout: 10_000 });
    await browser.keys(["Control", "a"]);
    await browser.keys("Backspace");
    await browser.keys("Escape");
    await waitForFileText(file("Emptied here"), (body) => !body.includes("a word"));
    proof.steps.push(`emptied in Tine: file is ${JSON.stringify(fs.readFileSync(file("Emptied here"), "utf8"))}`);
    const typed = await clickLastBulletAndType(".main-content", "typed after emptying");
    await expectPersisted(file("Emptied here"), "typed after emptying", `emptied page took typing (editor held ${JSON.stringify(typed)})`);
  });

  // 2. A routed page whose file is 0 bytes on disk from the start. Before it,
  //    another 0-byte page is only clicked: that writes nothing (the later
  //    persisted edit shows saves have run).
  await scenario("0-byte page", async () => {
    await openPageByName(browser, "Clicked only");
    await browser.$(".main-content [data-block-id] .block-content-wrapper").click();
    await browser.keys("Escape");
    await openPageByName(browser, "Zero bytes");
    const typed = await clickLastBulletAndType(".main-content", "zero byte text");
    await expectPersisted(file("Zero bytes"), "zero byte text", `0-byte page took typing (editor held ${JSON.stringify(typed)})`);
    const untouched = fs.readFileSync(file("Clicked only"), "utf8");
    if (untouched !== "") throw new Error(`clicking a 0-byte page's bullet wrote ${JSON.stringify(untouched)}`);
    proof.steps.push("clicking a 0-byte page's bullet without typing left its file empty");
  });

  // 3. A page whose file holds only page properties (almmos's Windows report).
  await scenario("properties-only page", async () => {
    await openPageByName(browser, "Properties only");
    const typed = await clickLastBulletAndType(".main-content", "body under properties");
    await expectPersisted(file("Properties only"), "body under properties", `properties-only page took typing (editor held ${JSON.stringify(typed)})`,
      (body) => body.startsWith("tags:: alpha"));
  });

  // 4. An Org page whose file is 0 bytes.
  await scenario("0-byte Org page", async () => {
    await openPageByName(browser, "Org empty");
    const typed = await clickLastBulletAndType(".main-content", "org body");
    await expectPersisted(path.join(GRAPH, "pages", "Org empty.org"), "org body", `0-byte Org page took typing (editor held ${JSON.stringify(typed)})`);
  });

  // 5. Today's journal, 0 bytes, in the Journals feed.
  await scenario("0-byte journal in the feed", async () => {
    await openJournals(browser);
    const typed = await clickLastBulletAndType(".main-content .page-section:first-of-type", "journal text");
    await expectPersisted(TODAY_FILE, "journal text", `0-byte journal in the feed took typing (editor held ${JSON.stringify(typed)})`);
  });

  // 6. A 0-byte page opened in the right sidebar, the main pane elsewhere.
  await scenario("0-byte page in the right sidebar", async () => {
    await openPageByName(browser, "Sidebar zero");
    await browser.execute(() => document.querySelector("h1.page-title")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, shiftKey: true })));
    await browser.$(".right-sidebar [data-block-id]").waitForExist({ timeout: 10_000 });
    await openJournals(browser);
    const typed = await clickLastBulletAndType(".right-sidebar", "sidebar text");
    await expectPersisted(file("Sidebar zero"), "sidebar text", `0-byte page in the right sidebar took typing (editor held ${JSON.stringify(typed)})`);
  });

  if (failures.length) throw new Error(failures.join(" || "));
  console.log("PASS", JSON.stringify(proof));
} catch (error) {
  proof.observed = String(error);
  proof.classification = "ambiguous; inspect DOM and driver log";
  if (browser) {
    try { await browser.saveScreenshot(path.join(TMP, "failure.png")); } catch {}
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  console.error("FAIL", JSON.stringify(proof));
  process.exitCode = 1;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}

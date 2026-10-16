// GH #243/#412/#494: native view, select-all and query-result drag boundaries.
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const root = path.resolve(import.meta.dirname, "..");
const testedCommit = execFileSync("rtk", ["proxy", "git", "rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const selectionProofs = [];
const out = process.env.E2E_ARTIFACT_DIR ?? path.join(root, "artifacts/sw-r2-residue/native");
fs.mkdirSync(out, { recursive: true });
const graph = fs.mkdtempSync(path.join(out, "graph-"));
for (const dir of ["pages", "journals", "logseq", "xdg/data", "xdg/config", "xdg/cache"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
const highlights = "a ==one== b ^^two^^ c";
fs.writeFileSync(path.join(graph, "pages/Residue.md"), `- ${highlights}\n- \`\`\`sh\n  echo hello\n  \`\`\`\n  - code child\n- before code\n  \`\`\`js\n  echo mixed\n  \`\`\`\n  after code\n- {{query (task TODO)}}\n  tine.view:: list\n- Drop here\n`);
fs.writeFileSync(path.join(graph, "pages/Source.md"), "- TODO native drag source\n- Source sibling\n");
fs.writeFileSync(path.join(graph, "pages/Org.org"), "* a ^^org highlight^^ b\n* #+BEGIN_SRC sh\necho org\n#+END_SRC\n");
const date = new Date();
const day = `${date.getFullYear()}_${String(date.getMonth() + 1).padStart(2, "0")}_${String(date.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(graph, `journals/${day}.md`), `- ${highlights}\n`);
const previous = new Date(date);
previous.setDate(previous.getDate() - 1);
const orgDay = `${previous.getFullYear()}_${String(previous.getMonth() + 1).padStart(2, "0")}_${String(previous.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(graph, `journals/${orgDay}.org`), "* a ^^journal org^^ b\n");
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(graph, "xdg/data"), XDG_CONFIG_HOME: path.join(graph, "xdg/config"), XDG_CACHE_HOME: path.join(graph, "xdg/cache"), WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER ?? "tauri-driver", ["--port", "4682", "--native-port", "4683", "--native-driver", process.env.WEBKIT_DRIVER ?? "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log] });
let browser, stage = "startup";
const assert = (ok, message) => { if (!ok) throw Error(message); };
try {
  await sleep(1500);
  browser = await remote({ hostname: "127.0.0.1", port: 4682, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: process.env.TINE_APP ?? path.join(root, "target/debug/tine") } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1100, 900);
  await browser.$(".page-title").waitForExist({ timeout: 20000 });
  const search = async name => {
    await browser.keys(["Control", "k"]);
    const input = await browser.$(".switcher-input");
    await input.waitForExist({ timeout: 5000 });
    await input.setValue(name);
    await browser.waitUntil(() => browser.execute(name => [...document.querySelectorAll(".switcher-row")].some(row => row.querySelector(".switcher-name")?.textContent.trim() === name), name), { timeout: 10000 });
    await browser.execute(name => [...document.querySelectorAll(".switcher-row")].find(row => row.querySelector(".switcher-name")?.textContent.trim() === name).dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })), name);
    await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === name, { timeout: 10000 });
  };
  const marks = async expected => {
    await browser.waitUntil(() => browser.execute(expected => JSON.stringify([...document.querySelectorAll(".block-content mark")].map(el => el.textContent)) === JSON.stringify(expected), expected), { timeout: 10000, timeoutMsg: "view-mode highlight text missing" });
    assert(await browser.execute(() => [...document.querySelectorAll(".block-content mark")].every(el => { const c = getComputedStyle(el); return c.backgroundColor !== "rgba(0, 0, 0, 0)" && c.backgroundColor !== "transparent"; })), "highlight has no painted background");
  };
  stage = "GH #243 Markdown and Org journal highlights";
  await marks(["one", "two", "journal org"]);
  console.log(`PASS ${stage}`);
  await search("Residue");
  stage = "GH #243 routed Markdown highlights";
  await marks(["one", "two"]);
  console.log(`PASS ${stage}`);
  const codeSelection = async (body, card = 0) => {
    await (await browser.$$("pre.code-block code"))[card].click();
    const editor = await browser.$("textarea.block-editor");
    await editor.waitForExist({ timeout: 5000 });
    await browser.keys(["Control", "a"]);
    const selection = await browser.execute(() => { const ta = document.querySelector("textarea.block-editor"); return { text: ta.value.slice(ta.selectionStart, ta.selectionEnd), focused: document.activeElement === ta }; });
    assert(selection.text === body && selection.focused, `native first Ctrl+A: ${JSON.stringify(selection)}`);
    await browser.keys(["Control", "a"]);
    await browser.keys(["Control", "c"]);
    const blockSelection = await browser.execute(async () => ({ editing: !!document.querySelector("textarea.block-editor"), copied: await window.__TAURI_INTERNALS__.invoke("plugin:clipboard-manager|read_text") }));
    const fence = body === "echo org" ? "#+BEGIN_SRC" : body === "echo mixed" ? "```js" : "```sh";
    assert(!blockSelection.editing && blockSelection.copied.includes(body) && blockSelection.copied.includes(fence), `second Ctrl+A did not select the complete source: ${JSON.stringify(blockSelection)}`);
    if (body === "echo hello") assert(blockSelection.copied.includes("code child"), "second Ctrl+A omitted the code block's child");
    if (body === "echo mixed") assert(blockSelection.copied.includes("before code") && blockSelection.copied.includes("after code"), "second Ctrl+A omitted prose around the code");
    selectionProofs.push({ body, first: selection, second: blockSelection });
    await browser.keys("Escape");
    console.log(`PASS ${stage}`);
  };
  stage = "GH #412 Markdown native Ctrl+A ladder";
  await codeSelection("echo hello");
  stage = "GH #412 mixed Markdown native Ctrl+A ladder";
  await codeSelection("echo mixed", 1);
  await search("Org");
  stage = "GH #243 routed Org highlight";
  await marks(["org highlight"]);
  console.log(`PASS ${stage}`);
  stage = "GH #412 Org native Ctrl+A ladder";
  await codeSelection("echo org");
  await search("Source"); // Make the query result's source a loaded edit owner.
  await search("Residue");
  const drag = async alt => {
    // Query results render asynchronously after navigation; wait for them instead of sampling once.
    let points = null;
    await browser.waitUntil(async () => (points = await browser.execute(() => {
      const hit = [...document.querySelectorAll(".query-group .ls-block, .query-results .ls-block, .ref-blocks .ls-block")].find(el => el.textContent.includes("native drag source"));
      const target = [...document.querySelectorAll(".ls-block")].find(el => el.querySelector(":scope > .block-main .block-content")?.textContent.trim() === "Drop here");
      if (!hit || !target) return null;
      const a = hit.querySelector(".bullet-container").getBoundingClientRect(), b = target.getBoundingClientRect();
      return { from: { x: Math.round(a.left + a.width / 2), y: Math.round(a.top + a.height / 2) }, to: { x: Math.round(b.left + 15), y: Math.round(b.bottom - 2) } };
    })) !== null, { timeout: 15000, interval: 200 }).catch(() => {});
    assert(points, "query result bullet / drop target missing");
    if (alt) await browser.performActions([{ type: "key", id: "modifier", actions: [{ type: "keyDown", value: "\uE00A" }] }]);
    await browser.performActions([{ type: "pointer", id: "drag", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 0, origin: "viewport", ...points.from }, { type: "pointerDown", button: 0 },
      { type: "pointerMove", duration: 600, origin: "viewport", ...points.to }, { type: "pause", duration: 150 }, { type: "pointerUp", button: 0 },
    ] }]);
    await browser.releaseActions();
  };
  stage = "GH #494 Alt query drag saves a reference without moving the source";
  await drag(true);
  let savedId;
  await browser.waitUntil(async () => {
    const source = fs.readFileSync(path.join(graph, "pages/Source.md"), "utf8");
    savedId = await browser.execute(() => document.querySelector(".block-ref")?.getAttribute("data-block-ref"));
    return savedId && source.includes(`id:: ${savedId}`) && source.includes("TODO native drag source") && fs.readFileSync(path.join(graph, "pages/Residue.md"), "utf8").includes(`((${savedId}))`);
  }, { timeout: 15000, timeoutMsg: "Alt-drag failed to preserve source and publish its reference" });
  assert(await browser.$(".block-ref").isExisting(), "inserted reference is not visible");
  console.log(`PASS ${stage}`);
  stage = "GH #494 ordinary query drag moves the result";
  await drag(false);
  await browser.waitUntil(() => !fs.readFileSync(path.join(graph, "pages/Source.md"), "utf8").includes("TODO native drag source") && fs.readFileSync(path.join(graph, "pages/Residue.md"), "utf8").includes("TODO native drag source"), { timeout: 15000 });
  assert(fs.readFileSync(path.join(graph, "pages/Residue.md"), "utf8").includes("{{query (task TODO)}}"), "query host changed");
  console.log(`PASS ${stage}`);
  await search("Source");
  await search("Residue");
  await browser.$(".block-ref").waitForExist({ timeout: 10000 });
  console.log("PASS moved source and block reference remain visible after page navigation");
  fs.writeFileSync(path.join(out, "result.json"), JSON.stringify({ outcome: "pass", testedCommit, graph, savedId, selectionProofs }, null, 2));
} catch (error) {
  if (browser) { try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.execute(() => document.documentElement.outerHTML)); } catch {} }
  try { execFileSync("rtk", ["proxy", "import", "-window", "root", path.join(out, "failure.png")]); } catch {}
  fs.writeFileSync(path.join(out, "result.json"), JSON.stringify({ outcome: "fail", testedCommit, classification: "ambiguous", expected: stage, observed: String(error), graph, selectionProofs }, null, 2));
  throw error;
} finally {
  if (browser) await browser.deleteSession();
  driver.kill();
  fs.closeSync(log);
}

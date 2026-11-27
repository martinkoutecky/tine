// GF4: native cold first-body timing and late-day merge with an open editor.
// Run under xvfb-run; E2E_BASELINE=1 records the unchanged app's timing only.
// The merge gate requires E2E_HOLD_LOAD=1 and tine-store/test-faults.
// GH #987: the click-opened editor holds still within 1 px in every rendered frame
// (tightened from a 2 px final read). E2E_EDITOR=textarea runs the classic editor;
// E2E_TYPE=1 also types while the late days land (the text must be saved); E2E_DIST
// names the build's frontend (with source maps) to label each scroll writer.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import { editorSelector, withEditor } from "./lib/e2e-block-editor.mjs";
import { armingProbe, heldEditorEvidence, labelReads, labelWrites, readCounts, waitQuiet, watchHeldEditor } from "./lib/e2e-held-editor.mjs";
import { seedClassicEditorProfile } from "./lib/e2e-classic-profile.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { generateRealisticGraph } from "./generate-realistic-graph.mjs";

const root = process.cwd();
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const hold = process.env.E2E_HOLD_LOAD === "1";
if (!hold && process.env.E2E_BASELINE !== "1") throw new Error("The journal merge gate requires E2E_HOLD_LOAD=1 and an app built with --features custom-protocol,tine-store/test-faults (TINE_APP); it is not a linux-release scenario");
const out = path.resolve(process.env.E2E_ARTIFACT_DIR || "artifacts/journals-today/native");
fs.mkdirSync(out, { recursive: true });
const trial = fs.mkdtempSync(path.join(out, "trial-"));
const graph = path.join(trial, "graph");
const manifest = await generateRealisticGraph({ root: graph, pages: 10000, journals: 0, seed: 543 });
const date = (ago) => { const d = new Date(); d.setDate(d.getDate() - ago); return d; };
const stem = (d) => `${d.getFullYear()}_${String(d.getMonth() + 1).padStart(2, "0")}_${String(d.getDate()).padStart(2, "0")}`;
const iso = (d) => stem(d).replaceAll("_", "-");
const title = (d) => `${d.toLocaleString("en-US", { month: "short" })} ${d.getDate()}${[11, 12, 13].includes(d.getDate()) ? "th" : ({ 1: "st", 2: "nd", 3: "rd" }[d.getDate() % 10] || "th")}, ${d.getFullYear()}`;
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
for (const ago of [0, 4, 6]) fs.writeFileSync(path.join(graph, "journals", `${stem(date(ago))}.md`), Array.from({ length: 80 }, (_, row) => `- GF4 body ${ago} row ${row}\n`).join(""));
fs.writeFileSync(path.join(graph, "pages/GF4 mentions.md"), `- [[${iso(date(1))}]] [[${iso(date(3))}]] [[2099-01-03]]\n`);
const pause = path.join(graph, ".tine-test-pause-load");
if (hold) fs.writeFileSync(pause, "");
for (const name of ["data", "config", "cache"]) fs.mkdirSync(path.join(trial, name));
const classic = process.env.E2E_EDITOR === "textarea";
const typing = process.env.E2E_TYPE === "1";
if (classic) seedClassicEditorProfile({ E2E_CLASSIC_PROFILE: "1", XDG_DATA_HOME: path.join(trial, "data") });
const HELD_PX = 1;
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(trial, "data"), XDG_CONFIG_HOME: path.join(trial, "config"), XDG_CACHE_HOME: path.join(trial, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4877);
const log = fs.openSync(path.join(trial, "driver.log"), "w");
const driver = spawn("dbus-run-session", ["--", process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver"), "--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
await sleep(500);
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"), manifest, hold, loadavg: os.loadavg(), journey: "cold Journals first body and referenced-day merge", editor: classic ? "textarea" : "cm6", typing, expected: "file-backed body paints before Ready; late days retain editor and viewport", steps: [] };
let browser;
const started = performance.now();
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  proof.sessionMs = performance.now() - started;
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".main-content .block-content-wrapper")].some(el => el.textContent.includes("GF4 body 0"))), { timeout: hold ? 10000 : 120000, interval: 50 });
  proof.firstBodyMs = performance.now() - started;
  await browser.saveScreenshot(path.join(trial, "first-body.png"));
  proof.steps.push("today body visible");
  if (process.env.E2E_BASELINE !== "1") {
    // The older file-backed day (4 days ago), picked by its title: the feed wraps every day in
    // `.journal-feed-day`, so a positional `.page-section:nth-child(2)` no longer matches, and
    // would name a different day once the fileless days are merged in.
    const older = await browser.$(`//*[contains(@class,"main-content")]//*[contains(@class,"page-section")][.//h1[contains(@class,"journal-title")][normalize-space()="${title(date(4))}"]]//*[contains(@class,"block-content-wrapper")][contains(.,"GF4 body 4 row 0")]`);
    await older.scrollIntoView({ block: "center" });
    await older.click();
    const editor = await browser.$(editorSelector(".main-content"));
    await editor.waitForExist({ timeout: 10000 });
    await withEditor(browser, ".main-content", driver => {
      window.__gf4OpeningSettled = false;
      const editor = driver.element;
      const scroller = editor.closest(".main-content");
      let previous, stable = 0;
      const sample = () => {
        if (!editor.isConnected) return;
        const signature = `${editor.getBoundingClientRect().top}|${scroller.scrollTop}|${scroller.scrollHeight}|${editor.clientHeight}`;
        stable = signature === previous && driver.focused && driver.value === "GF4 body 4 row 0" ? stable + 1 : 0;
        previous = signature;
        if (stable >= 2) window.__gf4OpeningSettled = true;
        else requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    });
    await browser.waitUntil(() => browser.execute(() => window.__gf4OpeningSettled),
      { timeout: 10000, interval: 50, timeoutMsg: "the known journal editor did not finish opening" });
    const before = await withEditor(browser, ".main-content", driver => {
      const editor = driver.element;
      window.__gf4Editor = editor;
      const scroller = editor.closest(".main-content");
      window.__gf4Frames = [];
      const sample = () => {
        const frame = { t: performance.now(), top: editor.getBoundingClientRect().top, scroll: scroller.scrollTop, days: document.querySelectorAll("h1.journal-title").length };
        const prev = window.__gf4Frames.at(-1);
        if (!prev || frame.top !== prev.top || frame.scroll !== prev.scroll || frame.days !== prev.days) window.__gf4Frames.push(frame);
        if (editor.isConnected) requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
      return { value: driver.value, top: editor.getBoundingClientRect().top, days: document.querySelectorAll("h1.journal-title").length };
    });
    proof.before = before;
    if (before.days !== 3 || before.value !== "GF4 body 4 row 0") throw new Error(`merge setup was not held: ${JSON.stringify(before)}`);
    if (await withEditor(browser, ".main-content", editor => editor?.kind) !== (classic ? "textarea" : "cm6")) throw new Error("the requested editor engine is not the one mounted");
    await watchHeldEditor(browser);
    proof.armingProbe = await armingProbe(browser);
    // Collected, not thrown at once, so one run also records the late-day merge.
    const failures = [];
    if (Math.abs(proof.armingProbe.grown) > HELD_PX || Math.abs(proof.armingProbe.shrunk) > HELD_PX) failures.push(`arming probe: the click-opened editor moved: ${JSON.stringify({ ...proof.armingProbe, writes: undefined })}`);
    else proof.steps.push("arming probe: a 16 px spacer above, grown and removed, left the editor within 1 px");
    const probed = await readCounts(browser);
    proof.probeReads = labelReads(probed, process.env.E2E_DIST || "dist");
    if (hold) fs.unlinkSync(pause);
    if (typing) {
      await withEditor(browser, ".main-content", editor => editor.select(editor.length));
      // One key per action: in one action batch WebKitWebDriver sent only the first space (no keydown for the rest).
      for (const key of " typed while days land") await browser.keys([key]);
    }
    await browser.waitUntil(() => browser.execute(name => [...document.querySelectorAll("h1.journal-title")].some(el => el.textContent.includes(name)), title(date(1))), { timeout: 120000, interval: 100 });
    await browser.waitUntil(() => browser.execute(names => names.every(name => {
      const section = [...document.querySelectorAll(".page-section")].find(el => el.querySelector("h1.journal-title")?.textContent.trim() === name);
      return section?.querySelector(".references-count")?.textContent.trim() === "1";
    }), [title(date(1)), title(date(3))]), { timeout: 30000, interval: 100, timeoutMsg: "inserted days' linked references did not finish" });
    // Wait until the editor and the pane have been still for 30 rendered frames, then read once; every
    // rendered frame since the watch began (after its corrections) is also checked against 1 px below.
    await waitQuiet(browser);
    const after = await withEditor(browser, ".main-content", editor => {
      return { sameEditor: editor?.element === window.__gf4Editor, focused: editor?.focused, value: editor?.value, top: editor?.element.getBoundingClientRect().top,
        titles: [...document.querySelectorAll("h1.journal-title")].map(el => el.textContent.trim()) };
    });
    proof.viewportFrames = await browser.execute(() => window.__gf4Frames);
    const evidence = await heldEditorEvidence(browser);
    proof.writes = labelWrites(evidence.writes, process.env.E2E_DIST || "dist");
    proof.natives = evidence.natives;
    proof.heldFrames = evidence.frames;
    proof.reads = labelReads(evidence.reads, process.env.E2E_DIST || "dist", probed);
    const moved = evidence.frames.filter(frame => Math.abs(frame.post) > HELD_PX);
    if (moved.length) failures.push(`the editor left its place in ${moved.length} rendered frames: ${JSON.stringify(moved.slice(0, 5))}`);
    const value = typing ? "GF4 body 4 row 0 typed while days land" : before.value;
    if (!after.sameEditor || !after.focused || after.value !== value || Math.abs(after.top - before.top) > HELD_PX) failures.push(`editor/viewport lost: ${JSON.stringify({ before, after })}`);
    if (failures.length) throw new Error(failures.join("\n"));
    if (typing) await waitForFileText(path.join(graph, "journals", `${stem(date(4))}.md`), text => text.includes(value), { timeoutMs: 20000 });
    const expected = [0, 1, 3, 4, 6].map(ago => title(date(ago)));
    if (JSON.stringify(after.titles) !== JSON.stringify(expected)) throw new Error(`day order: ${JSON.stringify(after.titles)}`);
    if (fs.readdirSync(path.join(graph, "journals")).length !== 3) throw new Error("merge created a journal file");
    proof.steps.push("late fileless days in date order; same focused editor and viewport; no journal writes");
    await browser.saveScreenshot(path.join(trial, "merged.png"));
    await browser.$(".nav-all-pages-open").click();
    await browser.$(".all-pages-filter").waitForExist({ timeout: 10000 });
    const readyStarted = performance.now();
    await browser.execute(() => [...document.querySelectorAll(".nav-item")].find(el => el.textContent.trim() === "Journals")?.click());
    await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".main-content .block-content-wrapper")].some(el => el.textContent.includes("GF4 body 0"))), { timeout: 10000, interval: 50 });
    proof.readyFirstBodyMs = performance.now() - readyStarted;
    await browser.waitUntil(() => browser.execute(expected => JSON.stringify([...document.querySelectorAll("h1.journal-title")].map(el => el.textContent.trim())) === JSON.stringify(expected), expected), { timeout: 10000, interval: 50 });
    proof.readyCompletedMs = performance.now() - readyStarted;
    proof.steps.push("already-Ready navigation away and back retains complete day ordering");
    await browser.saveScreenshot(path.join(trial, "ready-return.png"));
  }
  console.log("PASS", JSON.stringify(proof));
} catch (error) {
  proof.observed = String(error); proof.classification = "ambiguous; inspect screenshot and driver log";
  if (browser) {
    if (!proof.writes) await heldEditorEvidence(browser).then(evidence => {
      proof.writes = labelWrites(evidence.writes, process.env.E2E_DIST || "dist");
      proof.natives = evidence.natives; proof.heldFrames = evidence.frames; proof.heldEditor = { connected: evidence.connected, sameEditor: evidence.sameEditor, top: evidence.top };
    }).catch(() => {});
    proof.viewportFrames = await browser.execute(() => window.__gf4Frames).catch(() => []);
    proof.editorAtFailure = await withEditor(browser, ".main-content", editor => {
      return { sameEditor: editor?.element === window.__gf4Editor, focused: editor?.focused,
        value: editor?.value, top: editor?.element.getBoundingClientRect().top };
    }).catch(() => null);
    await browser.saveScreenshot(path.join(trial, "failure.png")).catch(() => {});
    fs.writeFileSync(path.join(trial, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  if (hold && fs.existsSync(pause)) fs.unlinkSync(pause);
  fs.writeFileSync(path.join(trial, "proof.json"), JSON.stringify(proof, null, 2));
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}

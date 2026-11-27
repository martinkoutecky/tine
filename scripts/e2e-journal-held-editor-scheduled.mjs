// GH #987: a click-opened editor in the Journals feed holds still while content above it changes.
// The user types into one block of an older day, then clicks a second block of that day (a trusted
// WebDriver click); a 16 px test spacer above is grown and removed (the arming probe); then a disk edit
// changes content above. E2E_CHANGE selects it:
//   grow (default)      another page gains two scheduled tasks: today's Scheduled section reruns, taller;
//   shrink              that page loses two of its three scheduled tasks: the section reruns, shorter;
//   remove-reachable    a disk edit removes all but the first block of the preceding day (29 of 30 rows),
//                       with enough offset left to keep the editor in place;
//   remove-unreachable  the same with too little offset: the pane scrolls to the top (the nearest place);
//   images              four image blocks are added to the preceding day and load late (an image burst).
// In every rendered frame (after its corrections) the editor stays within 1 px, except remove-unreachable,
// whose editor must end at the nearest reachable place: the top of the pane's range.
// Run under xvfb-run with TINE_APP; E2E_EDITOR=textarea runs the classic editor; E2E_LONG=1 makes the
// clicked block about 1500 characters; E2E_DIST names the build's frontend (with source maps) to label
// each scroll writer and each read of the editor's box (the work by source).
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { editorSelector, withEditor } from "./lib/e2e-block-editor.mjs";
import { armingProbe, heldEditorEvidence, labelReads, labelWrites, readCounts, waitQuiet, watchHeldEditor } from "./lib/e2e-held-editor.mjs";
import { seedClassicEditorProfile } from "./lib/e2e-classic-profile.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { installAccountHome } from "./lib/e2e-account-home.mjs";

const change = process.env.E2E_CHANGE || "grow";
if (!["grow", "shrink", "remove-reachable", "remove-unreachable", "images"].includes(change)) throw new Error(`unknown E2E_CHANGE ${change}`);
const classic = process.env.E2E_EDITOR === "textarea";
const HELD_PX = 1;
const long = process.env.E2E_LONG === "1";
const dist = process.env.E2E_DIST || "dist";
const removal = change.startsWith("remove");
const app = path.resolve(process.env.TINE_APP || "target/debug/tine");
const out = path.resolve(process.env.E2E_ARTIFACT_DIR || "artifacts/journal-held-editor-scheduled");
fs.mkdirSync(out, { recursive: true });
const trial = fs.mkdtempSync(path.join(out, `${change}-`));
const graph = path.join(trial, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const name of ["data", "config", "cache"]) fs.mkdirSync(path.join(trial, name));
installAccountHome(trial);
if (classic) seedClassicEditorProfile({ E2E_CLASSIC_PROFILE: "1", XDG_DATA_HOME: path.join(trial, "data") });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
const day = ago => { const d = new Date(); d.setDate(d.getDate() - ago); return d; };
const stem = d => `${d.getFullYear()}_${String(d.getMonth() + 1).padStart(2, "0")}_${String(d.getDate()).padStart(2, "0")}`;
const file = ago => path.join(graph, "journals", `${stem(day(ago))}.md`);
const scheduled = d => `<${stem(d).replaceAll("_", "-")} ${d.toLocaleString("en-US", { weekday: "short" })}>`;
// Today is short for the removals, so deleting day 1 leaves too little offset in the unreachable case.
const rows = ago => ago === 0 && removal ? 3 : 30;
const plans = n => Array.from({ length: n }, (_, i) => `- TODO Plan ${i + 1}\n  SCHEDULED: ${scheduled(day(0))}\n`).join("");
const plansFile = path.join(graph, "pages", "Plans.md");
if (!removal) fs.writeFileSync(plansFile, plans(change === "shrink" ? 3 : 1));
// The clicked block of day 2 and where it is put in the pane before the click (px below the pane top).
const clicked = change === "remove-unreachable" ? 0 : 12;
// The long classic block: the caret guard's mirror layout is proportional to the value's length.
const filler = Array.from({ length: 150 }, (_, i) => `word${i}`).join(" ");
function text(ago, row) { return `Held day ${ago} row ${row}${long && ago === 2 && row === clicked ? ` ${filler}` : ""}`; }
const images = [1, 2, 3, 4].map(n => `p${n}.png`);
if (change === "images") {
  fs.mkdirSync(path.join(graph, "assets"));
  for (const name of images) fs.copyFileSync(path.resolve("src-tauri/icons/128x128@2x.png"), path.join(graph, "assets", name));
}
// Unreachable: placed low, the offset (about 730 px) is less than day 1's removed height (about 928 px).
const placeAt = change === "remove-unreachable" ? 700 : 300;
for (const ago of [0, 1, 2, 3]) fs.writeFileSync(file(ago), Array.from({ length: rows(ago) }, (_, r) => `- ${text(ago, r)}\n`).join(""));

const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(trial, "data"), XDG_CONFIG_HOME: path.join(trial, "config"), XDG_CACHE_HOME: path.join(trial, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4887);
const log = fs.openSync(path.join(trial, "driver.log"), "w");
const driver = spawn("dbus-run-session", ["--", process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "/opt/cargo", "bin/tauri-driver"), "--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
await sleep(500); // tauri-driver's listen, as in e2e-journals-today.mjs
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), journey: "held Journals editor while content above changes", change, long, editor: classic ? "textarea" : "cm6", steps: [] };
const blockOf = (ago, row) => `//*[contains(@class,"main-content")]//*[contains(@class,"block-content-wrapper")][normalize-space()="${text(ago, row)}"]`;
let browser;
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await browser.$(blockOf(2, 29)).waitForExist({ timeout: 60000 });
  if (!removal) await browser.waitUntil(() => browser.execute(() => document.querySelector(".agenda-block")?.textContent.includes("Plan 1")),
    { timeout: 30000, interval: 100, timeoutMsg: "today's Scheduled section did not show the plan" });
  const place = (ago, row, y) => browser.execute((xpath, y) => {
    const el = document.evaluate(xpath, document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue;
    const scroller = el.closest(".main-content");
    scroller.scrollTop += el.getBoundingClientRect().top - scroller.getBoundingClientRect().top - y;
  }, blockOf(ago, row), y);
  const settled = value => browser.waitUntil(() => browser.executeAsync((value, done) => {
    const editor = document.activeElement;
    const scroller = editor?.closest?.(".main-content");
    if (!scroller) return done(false);
    const read = () => `${editor.getBoundingClientRect().top}|${scroller.scrollTop}|${scroller.scrollHeight}`;
    const first = read();
    requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(() => done(read() === first && (editor.value ?? editor.textContent) === value))));
  }, value), { timeout: 10000, interval: 50, timeoutMsg: `the editor of "${value}" did not settle` });

  // Type into one block (the #987 path), then click a second block of the same day.
  await place(2, 3, 200);
  await browser.waitUntil(() => browser.executeAsync(done => {
    const scroller = document.querySelector(".main-content");
    const first = `${scroller.scrollTop}|${scroller.scrollHeight}`;
    requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(() => done(first === `${scroller.scrollTop}|${scroller.scrollHeight}`))));
  }), { timeout: 10000, interval: 50, timeoutMsg: "the feed did not settle before the first click" });
  await browser.$(blockOf(2, 3)).click();
  await browser.$(editorSelector(".main-content")).waitForExist({ timeout: 10000 });
  await settled(text(2, 3));
  await withEditor(browser, ".main-content", editor => editor.select(editor.length));
  await browser.keys([..." typed"]);
  await waitForFileText(file(2), saved => saved.includes(`${text(2, 3)} typed`), { timeoutMs: 20000 });
  proof.steps.push("typed into one block and saved");
  await place(2, clicked, placeAt);
  await browser.$(blockOf(2, clicked)).click();
  await settled(text(2, clicked));
  if (await withEditor(browser, ".main-content", editor => editor?.kind) !== (classic ? "textarea" : "cm6")) throw new Error("the requested editor engine is not the one mounted");
  proof.baseline = await watchHeldEditor(browser);
  const failures = [];
  proof.armingProbe = await armingProbe(browser);
  if (Math.abs(proof.armingProbe.grown) > HELD_PX || Math.abs(proof.armingProbe.shrunk) > HELD_PX) failures.push(`arming probe: the click-opened editor moved: ${JSON.stringify({ ...proof.armingProbe, writes: undefined })}`);
  else proof.steps.push("arming probe: a 16 px spacer above, grown and removed, left the editor within 1 px");
  const probed = await readCounts(browser);
  proof.probeReads = labelReads(probed, dist);

  if (change === "grow") fs.writeFileSync(plansFile, plans(3));
  if (change === "shrink") fs.writeFileSync(plansFile, plans(1));
  if (removal) fs.writeFileSync(file(1), `- ${text(1, 0)}\n`);
  if (change === "images") fs.writeFileSync(file(1), images.map(name => `- ![${name}](../assets/${name})\n`).join("") + fs.readFileSync(file(1), "utf8"));
  await browser.waitUntil(() => browser.execute((change, gone) => {
    const agenda = document.querySelector(".agenda-block")?.textContent ?? "";
    if (change === "grow") return agenda.includes("Plan 3");
    if (change === "shrink") return !agenda.includes("Plan 2");
    if (change === "images") {
      const shown = [...document.querySelectorAll(".main-content img")].filter(img => /^p[1-4]\.png$/.test(img.alt)); // src is a blob URL
      return shown.length === 4 && shown.every(img => img.complete && img.naturalWidth > 0);
    }
    return ![...document.querySelectorAll(".block-content-wrapper")].some(el => el.textContent.trim() === gone);
  }, change, text(1, 29)), { timeout: 30000, interval: 100, timeoutMsg: "the change above did not reach the feed" });
  await waitQuiet(browser);
  const evidence = await heldEditorEvidence(browser);
  proof.writes = labelWrites(evidence.writes, dist);
  proof.reads = labelReads(evidence.reads, dist, probed);
  proof.natives = evidence.natives;
  proof.heldFrames = evidence.frames;
  proof.after = { connected: evidence.connected, sameEditor: evidence.sameEditor, top: evidence.top,
    scroll: await browser.execute(() => document.querySelector(".main-content").scrollTop) };
  if (!evidence.connected || !evidence.sameEditor) failures.push(`the clicked editor did not stay open and focused: ${JSON.stringify(proof.after)}`);
  if (change === "remove-unreachable") {
    // The day removed above was taller than the offset: the nearest place is the top of the range.
    if (proof.after.scroll > HELD_PX) failures.push(`the pane did not go to the nearest reachable place: ${JSON.stringify(proof.after)}`);
  } else {
    const moved = evidence.frames.filter(frame => Math.abs(frame.post) > HELD_PX);
    if (moved.length) failures.push(`the editor left its place in ${moved.length} rendered frames: ${JSON.stringify(moved.slice(0, 5))}`);
  }
  if (failures.length) throw new Error(failures.join("\n"));
  proof.steps.push(change === "remove-unreachable" ? "the pane went to the nearest reachable place" : "the editor stayed within 1 px in every rendered frame");
  console.log("PASS", JSON.stringify({ change, editor: proof.editor, trial }));
} catch (error) {
  proof.observed = String(error); proof.classification = "ambiguous; inspect screenshot, proof and driver log";
  if (browser) {
    if (!proof.writes) await heldEditorEvidence(browser).then(evidence => {
      proof.writes = labelWrites(evidence.writes, dist);
      proof.natives = evidence.natives; proof.heldFrames = evidence.frames; proof.after ??= { connected: evidence.connected, sameEditor: evidence.sameEditor, top: evidence.top };
    }).catch(() => {});
    await browser.saveScreenshot(path.join(trial, "failure.png")).catch(() => {});
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(trial, "proof.json"), JSON.stringify(proof, null, 2));
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}

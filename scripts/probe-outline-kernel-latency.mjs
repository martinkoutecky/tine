// Outline-kernel gate 2 (DESIGN §0, §8): a structural edit on the largest page of
// the anonymized graph costs <= 4 ms p95 end to end in the window, measured in
// WebKitGTK on the real app. Each sample times one synchronous keydown dispatch
// on the open editor: the key handler, page marshal, kernel call, store patch
// and Solid's synchronous DOM update. That bounds "marshal + kernel + patch
// apply" from above. Runs on a private copy of the graph; prints JSON.
//
//   xvfb-run -a -s "-screen 0 1920x1080x24" node scripts/probe-outline-kernel-latency.mjs
import { spawn, spawnSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const ANON = process.env.TINE_ANON || path.join(os.homedir(), "research/logseq-anonymized");
const PAGE = process.env.TINE_PROBE_PAGE || "VHI 5569";
const NEEDLE = process.env.TINE_PROBE_NEEDLE || "ohkzinropsmyn";
const ROUNDS = Number(process.env.TINE_PROBE_ROUNDS || 40);
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4498);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4499);
const TMP = "/tmp/tine-outline-kernel-latency";
const GRAPH = `${TMP}/graph`;

fs.rmSync(TMP, { recursive: true, force: true });
fs.mkdirSync(TMP, { recursive: true });
if (spawnSync("cp", ["-r", ANON, GRAPH]).status !== 0) throw new Error(`could not copy ${ANON}`);
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

// One synchronous keydown on the focused block editor. Returns the elapsed time
// and the editor's outline depth (its .ls-block ancestors) afterwards.
const press = (key, shift, caretAtEnd) => browser.execute((key, shift, caretAtEnd) => {
  const ta = document.activeElement;
  if (!(ta instanceof HTMLTextAreaElement)) return { error: "no focused editor" };
  if (caretAtEnd) ta.setSelectionRange(ta.value.length, ta.value.length);
  else ta.setSelectionRange(0, 0);
  const event = new KeyboardEvent("keydown", { key, shiftKey: shift, bubbles: true, cancelable: true });
  const t0 = performance.now();
  ta.dispatchEvent(event);
  const ms = performance.now() - t0;
  return { ms };
}, key, shift, caretAtEnd);

const editorState = () => browser.execute((needle) => {
  const ta = document.activeElement;
  if (!(ta instanceof HTMLTextAreaElement)) return null;
  let depth = 0;
  for (let el = ta.parentElement; el; el = el.parentElement) if (el.classList.contains("ls-block")) depth++;
  return { depth, value: ta.value, mine: ta.value.includes(needle) };
}, NEEDLE);

async function settle(predicate, what) {
  await browser.waitUntil(async () => predicate(await editorState()), { timeout: 5000, timeoutMsg: what });
  await sleep(30);
}

const stats = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  const q = (f) => s[Math.min(s.length - 1, Math.ceil(f * s.length) - 1)];
  return { n: s.length, p50: q(0.5), p95: q(0.95), max: s[s.length - 1] };
};

let browser;
try {
  browser = await remote({ hostname: "127.0.0.1", port: DRIVER_PORT, logLevel: "error", capabilities: { "tauri:options": { application: APP } } });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 60_000 });
  await openPageByName(browser, PAGE, { timeout: 30_000 });
  const blocks = await browser.execute(() => document.querySelectorAll(".ls-block").length);
  const clicked = await browser.execute((needle) => {
    const content = [...document.querySelectorAll(".ls-block .block-content")].find((el) => el.textContent.includes(needle));
    if (!content) return false;
    content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    content.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0 }));
    return true;
  }, NEEDLE);
  if (!clicked) throw new Error(`no block containing ${NEEDLE} on ${PAGE}`);
  await settle((s) => s?.mine, "the target block did not open its editor");
  const home = (await editorState()).depth;
  const samples = { indent: [], outdent: [], split: [], merge: [] };
  for (let i = 0; i < ROUNDS; i++) {
    samples.indent.push((await press("Tab", false, true)).ms);
    await settle((s) => s?.mine && s.depth === home + 1, `round ${i}: Tab did not indent`);
    samples.outdent.push((await press("Tab", true, true)).ms);
    await settle((s) => s?.mine && s.depth === home, `round ${i}: Shift+Tab did not outdent`);
    samples.split.push((await press("Enter", false, true)).ms);
    await settle((s) => s && !s.mine && s.value === "", `round ${i}: Enter did not split`);
    samples.merge.push((await press("Backspace", false, false)).ms);
    await settle((s) => s?.mine, `round ${i}: Backspace did not merge back`);
  }
  const result = { page: PAGE, renderedBlocks: blocks, rounds: ROUNDS, app: APP, ms: Object.fromEntries(Object.entries(samples).map(([k, v]) => [k, stats(v)])) };
  console.log(JSON.stringify(result, null, 1));
} finally {
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-td.pid); } catch {}
}

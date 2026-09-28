// GH #532 / og 11c: when the local day changes while the user is typing in the
// Journals feed, the new day appears at the top of the feed and the open editor
// keeps its textarea, unsaved text, selection and focus; the text then saves to
// the day it was typed in.
//
// The native app, its WebKit web process and the backend `journalFeedPage` day
// must all see the same clock, so the whole app runs under libfaketime with one
// RELATIVE offset (every process ticks in real time, shifted to ~75 s before
// UTC midnight). The journey fails closed when libfaketime is not available:
// set E2E_LIBFAKETIME to libfaketime.so.1. Monotonic clocks and file stat times
// stay real, so timers, WebDriver and the watcher are unaffected.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";

// Every Date below is UTC; re-exec with TZ set before the first Date is built.
if (process.env.TZ !== "UTC") {
  const child = spawn(process.execPath, [new URL(import.meta.url).pathname], { env: { ...process.env, TZ: "UTC" }, stdio: "inherit" });
  child.on("exit", (code) => process.exit(code ?? 1));
} else {
  await main();
}

async function main() {
  const FAKETIME_LIB = [process.env.E2E_LIBFAKETIME, "/usr/lib/x86_64-linux-gnu/faketime/libfaketime.so.1",
    "/aux/koutecky/logseq/.codex-deps/faketime/src/src/libfaketime.so.1"].find((file) => file && fs.existsSync(file));
  if (!FAKETIME_LIB) {
    console.error("FAIL: libfaketime.so.1 not found (set E2E_LIBFAKETIME); this journey cannot move the app's clock without it");
    process.exit(1);
  }
  const TMP = process.env.E2E_TMP_DIR || `/tmp/tine-journal-rollover-e2e-${process.pid}`;
  const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;
  const G = `${TMP}/graph`;
  const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
  const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
  const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4444);
  const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4445);
  const LEAD_SECONDS = Number(process.env.E2E_ROLLOVER_LEAD_S || 75);

  const pad = (n) => String(n).padStart(2, "0");
  const ordinal = (n) => `${n}${n % 100 >= 11 && n % 100 <= 13 ? "th" : ({ 1: "st", 2: "nd", 3: "rd" }[n % 10] || "th")}`;
  const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  const title = (d) => `${MONTHS[d.getUTCMonth()]} ${ordinal(d.getUTCDate())}, ${d.getUTCFullYear()}`; // default MMM do, yyyy
  const file = (d) => `${d.getUTCFullYear()}_${pad(d.getUTCMonth() + 1)}_${pad(d.getUTCDate())}.md`;

  // Fake "today" D is the real UTC date; the app starts LEAD_SECONDS before D's midnight.
  const real = new Date();
  const dayD = new Date(Date.UTC(real.getUTCFullYear(), real.getUTCMonth(), real.getUTCDate()));
  const dayNext = new Date(dayD.getTime() + 86_400_000);
  const offsetS = Math.round((dayNext.getTime() - LEAD_SECONDS * 1000 - Date.now()) / 1000);
  const TODAY_TEXT = "written before midnight";
  const TYPED = " and after";

  fs.rmSync(TMP, { recursive: true, force: true });
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${G}/${dir}`, { recursive: true });
  for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
  fs.writeFileSync(`${G}/logseq/config.edn`, "{}\n");
  fs.writeFileSync(`${G}/journals/${file(dayD)}`, `- ${TODAY_TEXT}\n`);
  fs.writeFileSync(`${G}/journals/${file(new Date(dayD.getTime() - 86_400_000))}`, "- the day before\n");

  const env = {
    ...process.env,
    TZ: "UTC",
    LD_PRELOAD: [FAKETIME_LIB, process.env.LD_PRELOAD].filter(Boolean).join(":"),
    FAKETIME: `${offsetS >= 0 ? "+" : ""}${offsetS}`,
    DONT_FAKE_MONOTONIC: "1",
    NO_FAKE_STAT: "1",
    FAKETIME_DONT_FAKE_MONOTONIC: "1",
    TINE_GRAPH: G,
    XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
  };
  fs.mkdirSync(ARTIFACTS, { recursive: true });
  const log = fs.openSync(path.join(ARTIFACTS, "journal-rollover-tauri-driver.log"), "w");
  const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
    env, stdio: ["ignore", log, log], detached: true,
  });
  await sleep(2500);

  let browser;
  let step = "launch";
  const sameEditor = (actual, wanted) => actual !== null && Object.keys(wanted).every((key) => actual[key] === wanted[key]);
  const feedState = () => browser.execute(() => {
    const editor = document.querySelector("textarea.block-editor");
    return {
      now: new Date().toISOString(),
      titles: [...document.querySelectorAll(".page-section h1.journal-title")].map((node) => node.textContent?.trim() ?? ""),
      editor: editor ? {
        mark: editor.dataset.rolloverMark ?? null,
        value: editor.value,
        start: editor.selectionStart,
        end: editor.selectionEnd,
        focused: document.activeElement === editor,
        day: editor.closest(".page-section")?.querySelector("h1.journal-title")?.textContent?.trim() ?? null,
      } : null,
      toasts: [...document.querySelectorAll(".toast")].map((node) => node.textContent),
    };
  });
  try {
    browser = await remote({
      hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    });
    step = "feed before midnight";
    await browser.waitUntil(async () => (await feedState()).titles[0] === title(dayD), {
      timeout: 30_000, interval: 200, timeoutMsg: `the feed never led with ${title(dayD)}`,
    });
    const before = await feedState();
    const msToMidnight = dayNext.getTime() - Date.parse(before.now);
    if (!(msToMidnight > 15_000)) throw new Error(`harness: the faked clock is too close to midnight to type first (${before.now})`);
    console.log(`app clock ${before.now}; ${Math.round(msToMidnight / 1000)} s to midnight`);

    step = "open an editor with unsaved text and a selection";
    const block = await browser.$(`//div[contains(concat(' ', normalize-space(@class), ' '), ' block-content ') and normalize-space(.) = '${TODAY_TEXT}']`);
    await block.waitForExist({ timeout: 10_000 });
    await block.click();
    await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
    await browser.keys(["End"]);
    await browser.keys(TYPED);
    const marked = await browser.execute(() => {
      const editor = document.querySelector("textarea.block-editor");
      if (!(editor instanceof HTMLTextAreaElement)) return false;
      editor.dataset.rolloverMark = "before-midnight";
      editor.setSelectionRange(2, 9);
      return true;
    });
    if (!marked) throw new Error("no editor to mark");
    const expected = { mark: "before-midnight", value: `${TODAY_TEXT}${TYPED}`, start: 2, end: 9, focused: true, day: title(dayD) };
    const editorBefore = (await feedState()).editor;
    if (!sameEditor(editorBefore, expected)) throw new Error(`editor setup differs: ${JSON.stringify(editorBefore)}`);

    step = "midnight passes";
    let after;
    await browser.waitUntil(async () => {
      after = await feedState();
      return after.titles[0] === title(dayNext);
    }, {
      timeout: msToMidnight + 30_000, interval: 250,
      timeoutMsg: `the feed did not add ${title(dayNext)} after midnight`,
    });
    console.log(`PASS: at ${after.now} the feed leads with ${title(dayNext)}: ${JSON.stringify(after.titles.slice(0, 3))}`);
    if (!after.titles.includes(title(dayD))) throw new Error(`the rollover dropped ${title(dayD)} from the feed: ${JSON.stringify(after.titles)}`);
    await sleep(1500);
    const settled = await feedState();
    if (!sameEditor(settled.editor, expected)) {
      throw new Error(`the open editor did not survive the rollover: ${JSON.stringify(settled.editor)} (expected ${JSON.stringify(expected)})`);
    }
    console.log("PASS: the same textarea kept its unsaved text, selection and focus");

    step = "the edit saves to the day it was typed in";
    await browser.keys(["Escape"]);
    await browser.waitUntil(() => fs.readFileSync(`${G}/journals/${file(dayD)}`, "utf8") === `- ${TODAY_TEXT}${TYPED}\n`, {
      timeout: 10_000, timeoutMsg: `the edit did not save to ${file(dayD)}: ${JSON.stringify(fs.readFileSync(`${G}/journals/${file(dayD)}`, "utf8"))}`,
    });
    const nextFile = `${G}/journals/${file(dayNext)}`;
    if (fs.existsSync(nextFile) && fs.readFileSync(nextFile, "utf8").includes(TYPED.trim())) {
      throw new Error(`the edit leaked into the new day's file: ${JSON.stringify(fs.readFileSync(nextFile, "utf8"))}`);
    }
    console.log(`PASS: the edit saved to ${file(dayD)} only`);
  } catch (error) {
    console.error(`FAIL at step: ${step}`);
    try { console.error("state:", JSON.stringify(await feedState())); } catch {}
    process.exitCode = 1;
    console.error(error);
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
  }
}

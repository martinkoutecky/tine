// GH #171: the native app and WebKit must agree on a deliberately skewed local
// civil day.  Future journals are feed-ineligible, but Ctrl-K can still open one
// without touching its file bytes.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const utcHour = new Date().getUTCHours();
const TZ = utcHour < 11 ? "Etc/GMT+12" : "Etc/GMT-14";
const G = "/tmp/tgraph-journal-future";
const XDG = "/tmp/txdg-journal-future";
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const driverPort = Number(process.env.E2E_DRIVER_PORT || 4444);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4445);
const pad = (n) => String(n).padStart(2, "0");
const stem = (d) => `${pad(d.getDate())}-${pad(d.getMonth() + 1)}-${d.getFullYear()}`;
const ordinal = (n) => {
  const mod100 = n % 100;
  return `${n}${mod100 >= 11 && mod100 <= 13 ? "th" : ({ 1: "st", 2: "nd", 3: "rd" }[n % 10] || "th")}`;
};
const customTitle = (d) => `${d.toLocaleDateString("en-US", { weekday: "long" })}, ${stem(d)}`;
const addDays = (d, n) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n, 12);

// A future journal's *title* legitimately remains in the sidebar's RECENT
// section after Ctrl-K opens it.  The invariant is narrower: it must not be
// rendered by the active Journals feed surface.  Keep the native assertion on
// that concrete surface rather than document.body, whose text also includes
// navigation chrome and overlays.
async function journalsFeedSurface(browser, futureTitle, futureContent) {
  return browser.execute(({ futureTitle, futureContent }) => {
    const journalsNav = [...document.querySelectorAll(".nav-item")]
      .find((item) => item.textContent?.trim() === "Journals");
    const scroller = document.querySelector('main.main-content[data-pane-id="main"]');
    const surface = scroller?.querySelector(":scope > .main-content-inner > .page");
    const feedText = surface?.textContent ?? "";
    const needles = [futureTitle, futureContent];
    const outsideMatches = needles.filter((needle) =>
      [...document.querySelectorAll("body *")].some((element) =>
        !surface?.contains(element) && element.textContent?.trim() === needle
      )
    );
    return {
      journalsIsActive: journalsNav?.classList.contains("active") ?? false,
      foundFeedSurface: !!surface,
      futureInFeed: needles.filter((needle) => feedText.includes(needle)),
      // Diagnostic only: these are permitted navigation/overlay residues, not
      // feed membership.  Reporting them makes the observation boundary clear.
      exactOutsideFeed: outsideMatches,
    };
  }, { futureTitle, futureContent });
}

// TZ must be in the process environment before this first Date construction.
if (process.env.TZ !== TZ) {
  const child = spawn(process.execPath, [new URL(import.meta.url).pathname], { env: { ...process.env, TZ }, stdio: "inherit" });
  child.on("exit", (code) => process.exit(code ?? 1));
} else {
  const local = new Date();
  const utcKey = local.getUTCFullYear() * 10000 + (local.getUTCMonth() + 1) * 100 + local.getUTCDate();
  const localKey = local.getFullYear() * 10000 + (local.getMonth() + 1) * 100 + local.getDate();
  console.log(`TZ=${TZ} utc=${local.toISOString()} utcKey=${utcKey} local=${local.toString()} localKey=${localKey}`);
  if (utcKey === localKey) throw new Error("skew-timezone precondition failed: local and UTC day match");
  const today = new Date(local.getFullYear(), local.getMonth(), local.getDate(), 12);
  const future = [addDays(today, 1), addDays(today, 2), addDays(today, 3)];
  const past = [addDays(today, -1), addDays(today, -2), addDays(today, -3), addDays(today, -4), addDays(today, -5)];
  fs.rmSync(G, { recursive: true, force: true });
  fs.mkdirSync(`${G}/journals`, { recursive: true });
  fs.mkdirSync(`${G}/pages`, { recursive: true });
  fs.mkdirSync(`${G}/logseq`, { recursive: true });
  fs.writeFileSync(`${G}/logseq/config.edn`, "{:journal/file-name-format \"dd-MM-yyyy\" :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n");
  // Three eligible journals are the first backend window.  Make each one
  // intrinsically tall so the release runner cannot auto-observe LoadMore
  // before this script proves a user-driven positive scroll.
  const tallJournal = (sentinel) => `${[`- ${sentinel}`, ...Array.from({ length: 36 }, (_, i) => `- tall seeded content ${i + 1}`)].join("\n")}\n`;
  fs.writeFileSync(`${G}/journals/${stem(today)}.md`, tallJournal("TODAY-SENTINEL"));
  for (const d of future) fs.writeFileSync(`${G}/journals/${stem(d)}.md`, `- FUTURE-${stem(d)}\n`);
  for (const d of past) fs.writeFileSync(`${G}/journals/${stem(d)}.md`, tallJournal(`PAST-${stem(d)}`));
  const futurePaths = future.map((d) => `${G}/journals/${stem(d)}.md`);
  const futureBytes = new Map(futurePaths.map((path) => [path, fs.readFileSync(path)]));
  fs.rmSync(XDG, { recursive: true, force: true });
  for (const d of ["data", "config", "cache"]) fs.mkdirSync(`${XDG}/${d}`, { recursive: true });
  const env = { ...process.env, TZ, TINE_GRAPH: G, XDG_DATA_HOME: `${XDG}/data`, XDG_CONFIG_HOME: `${XDG}/config`, XDG_CACHE_HOME: `${XDG}/cache`, WEBKIT_DISABLE_DMABUF_RENDERER: "1", LIBGL_ALWAYS_SOFTWARE: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", GDK_BACKEND: "x11" };
  const log = fs.openSync("/tmp/td-journal-future.log", "w");
  const td = spawn(TD, ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
  let browser;
  try {
    await sleep(2500);
    browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/", capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } }, logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000 });
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
    await sleep(1000);
    const body = await browser.execute(() => document.body.innerText);
    if (!body.includes("TODAY-SENTINEL")) throw new Error("real local-today content is absent from first Journals paint");
    if (future.some((d) => body.includes(`FUTURE-${stem(d)}`) || body.includes(customTitle(d)))) throw new Error("future journal leaked into Journals feed");
    if (!body.includes(`PAST-${stem(past[0])}`)) throw new Error("eligible past journal is absent from feed");
    const secondWindowSentinel = `PAST-${stem(past[3])}`;
    if (body.includes(secondWindowSentinel)) throw new Error("second backend window arrived before the deliberate scroll proof");
    const initialDays = ["TODAY-SENTINEL", `PAST-${stem(past[0])}`, `PAST-${stem(past[1])}`]
      .filter((sentinel) => body.includes(sentinel));
    if (initialDays.length !== 3) throw new Error(`unexpected initial eligible feed window: ${JSON.stringify(initialDays)}`);
    // LoadMore is an IntersectionObserver sentinel inside the actual scrolling
    // container.  Drive that container to a geometrically observed bottom and
    // dispatch its real scroll event; scrolling document.body is not a WebKit
    // feed interaction and can leave the sentinel outside the viewport.
    const scrollProof = await browser.execute(() => {
      const scroller = document.querySelector(".main-content");
      const sentinel = document.querySelector(".feed-sentinel");
      if (!(scroller instanceof HTMLElement) || !(sentinel instanceof HTMLElement)) return { ok: false };
      const before = scroller.scrollTop;
      const sentinelBefore = sentinel.getBoundingClientRect();
      const scrollerBox = scroller.getBoundingClientRect();
      scroller.scrollTop = scroller.scrollHeight;
      scroller.dispatchEvent(new Event("scroll", { bubbles: true }));
      const sentinelAfter = sentinel.getBoundingClientRect();
      return {
        ok: scroller.scrollHeight > scroller.clientHeight && scroller.scrollTop > before && sentinelAfter.top <= scrollerBox.bottom,
        before, after: scroller.scrollTop, clientHeight: scroller.clientHeight,
        scrollHeight: scroller.scrollHeight, sentinelTop: sentinelAfter.top, bottom: scrollerBox.bottom,
      };
    });
    if (!scrollProof.ok) throw new Error(`could not produce an observed bottom feed scroll: ${JSON.stringify(scrollProof)}`);
    await browser.waitUntil(async () => (await browser.execute(() => document.body.innerText)).includes(secondWindowSentinel), {
      timeout: 10_000, timeoutMsg: "load-more did not append the next eligible journal window",
    });
    const chronology = await browser.execute(() => document.body.innerText);
    const expectedEligibleSentinels = [
      "TODAY-SENTINEL",
      ...past.map((d) => `PAST-${stem(d)}`),
    ];
    const chronologyDiagnostics = expectedEligibleSentinels.map((sentinel) => {
      const indices = [];
      for (let index = chronology.indexOf(sentinel); index >= 0; index = chronology.indexOf(sentinel, index + sentinel.length)) {
        indices.push(index);
      }
      return { sentinel, indices, occurrences: indices.length };
    });
    const missingOrDuplicate = chronologyDiagnostics.filter(({ occurrences }) => occurrences !== 1);
    const orderedIndices = chronologyDiagnostics.map(({ indices }) => indices[0]);
    const newestFirst = orderedIndices.every((index, position) => position === 0 || index > orderedIndices[position - 1]);
    if (missingOrDuplicate.length || !newestFirst) {
      throw new Error(`eligible journal load-more chronology must render every expected day exactly once in newest-first order: ${JSON.stringify({
        expectedEligibleSentinels,
        missingOrDuplicate,
        orderedIndices,
        newestFirst,
      })}`);
    }
    await browser.keys(["Control", "k"]);
    const input = await browser.$(".switcher-input");
    await input.waitForExist({ timeout: 5_000 });
    const futureName = customTitle(future[0]);
    await input.setValue(futureName);
    await browser.waitUntil(() => browser.execute((wanted) => [...document.querySelectorAll(".switcher-section")].some((section) =>
      section.querySelector(".switcher-group-header > span:first-child")?.textContent?.trim() === "Pages"
      && [...section.querySelectorAll(".switcher-row:not(.block-result) .switcher-name")]
        .some((element) => element.textContent?.trim() === wanted)
    ), futureName), { timeout: 10_000, interval: 100, timeoutMsg: "Ctrl-K exact future-journal result did not appear" });
    // Select through the literal keyboard path: Ctrl-K, exact query, Return.
    await browser.keys(["Enter"]);
    await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === futureName, {
      timeout: 10_000, timeoutMsg: "Ctrl-K selected the wrong route instead of the future journal",
    });
    if (!(await browser.execute(() => document.body.innerText)).includes(`FUTURE-${stem(future[0])}`)) throw new Error("Ctrl-K route did not show intact future journal content");
    await browser.execute(() => {
      const journals = [...document.querySelectorAll(".nav-item")].find((item) => item.textContent?.trim() === "Journals");
      (journals instanceof HTMLElement ? journals : null)?.click();
    });
    await browser.waitUntil(async () => (await browser.execute(() => document.body.innerText)).includes("TODAY-SENTINEL"), {
      timeout: 10_000, timeoutMsg: "could not return to Journals after direct future-page access",
    });
    const futureTitle = customTitle(future[0]);
    const futureContent = `FUTURE-${stem(future[0])}`;
    const returned = await journalsFeedSurface(browser, futureTitle, futureContent);
    console.log(`Journals feed return diagnostic: ${JSON.stringify(returned)}`);
    if (!returned.journalsIsActive || !returned.foundFeedSurface) {
      throw new Error(`could not identify the active Journals feed surface after return: ${JSON.stringify(returned)}`);
    }
    if (returned.futureInFeed.length) {
      throw new Error(`future journal appeared inside the Journals feed after returning: ${JSON.stringify(returned)}`);
    }
    console.log("PASS: local-today feed membership, chronological load-more, custom formats, exact Ctrl-K future access, and return exclusion");
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    for (const [path, bytes] of futureBytes) {
      if (!bytes.equals(fs.readFileSync(path))) throw new Error(`future journal bytes changed during read-only feed scenario: ${path}`);
    }
  }
}

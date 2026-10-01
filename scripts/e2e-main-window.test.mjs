import test from "node:test";
import assert from "node:assert/strict";
import { chooseMainWindow, ensureMainWindow, isAuxiliaryWindow } from "./lib/e2e-main-window.mjs";

const APP = "http://tauri.localhost/";
const CAPTURE = "http://tauri.localhost/capture.html";

test("recognises Tine's auxiliary documents", () => {
  assert.equal(isAuxiliaryWindow(CAPTURE), true);
  assert.equal(isAuxiliaryWindow("http://tauri.localhost/capture.html?graph=x"), true);
  assert.equal(isAuxiliaryWindow(APP), false);
  assert.equal(isAuxiliaryWindow("http://tauri.localhost/index.html"), false);
});

test("switches away from the window windows-smoke was stranded on", () => {
  const windows = [{ handle: "a", url: CAPTURE }, { handle: "b", url: APP }];
  assert.equal(chooseMainWindow(windows, "a"), "b");
});

test("stays put when the session is already on the app", () => {
  const windows = [{ handle: "a", url: CAPTURE }, { handle: "b", url: APP }];
  assert.equal(chooseMainWindow(windows, "b"), null);
});

test("reports that there is no app window at all, rather than picking one", () => {
  assert.equal(chooseMainWindow([{ handle: "a", url: CAPTURE }], "a"), undefined);
});

// The breadth half of the fix. The first repair put `ensureMainWindow` in
// `e2e-pdf-logseq.mjs` alone, because that was the journey whose failure named
// the capture window. A later burn-in then failed three OTHER journeys on the
// same runner batch -- `page-properties` ("Quick Switcher did not open ...
// url: capture.html"), `print-security` ("the application search control never
// became clickable: missing") and `windows-core` ("shell, scroll pane and
// seeded block did not become ready") -- one cause wearing three costumes. So
// the requirement belongs to the whole suite, not to the journey that happened
// to report it first.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptsDir = path.dirname(fileURLToPath(import.meta.url));

/** The journey scripts the windows-smoke suite runs. */
export function windowsSmokeScripts(runnerSource) {
  const start = runnerSource.indexOf('"windows-smoke": [');
  if (start < 0) throw new Error("run-e2e.mjs no longer declares a windows-smoke suite");
  const end = runnerSource.indexOf("\n  ],", start);
  return [...new Set([...runnerSource.slice(start, end).matchAll(/"(scripts\/e2e-[^"]+\.mjs)"/g)]
    .map((match) => match[1]))].sort();
}

test("every windows-smoke journey points its session at the app window", () => {
  const runner = fs.readFileSync(path.join(scriptsDir, "run-e2e.mjs"), "utf8");
  const journeys = windowsSmokeScripts(runner);
  assert.ok(journeys.length >= 6, `expected the windows-smoke suite, saw ${journeys.length} journeys`);
  const missing = journeys.filter((journey) => {
    const source = fs.readFileSync(path.join(scriptsDir, "..", journey), "utf8");
    const sessions = [...source.matchAll(/browser = await remote\(\{[\s\S]*?\n  \}\);/g)];
    return sessions.length === 0 || sessions.some((session) =>
      !source.slice(session.index + session[0].length).startsWith("\n  await ensureMainWindow(browser);"));
  });
  assert.deepEqual(
    missing,
    [],
    "I-12: These windows-smoke journeys create a WebDriver session without calling ensureMainWindow(browser).\n"
    + "On Windows the driver can attach to the Quick Capture window, where every application\n"
    + "selector is legitimately absent, and the journey then reports its own missing element\n"
    + "rather than the wrong window. scripts/e2e-print-security.mjs is the exemplar call site.",
  );
});

function session(windows, current) {
  let active = current;
  const switched = [];
  return {
    getWindowHandles: async () => windows.map((window) => window.handle),
    getWindowHandle: async () => active,
    switchToWindow: async (handle) => { active = handle; switched.push(handle); },
    getUrl: async () => windows.find((window) => window.handle === active).url,
    active: () => active,
    switched,
  };
}

test("the real session selection leaves the attached capture window for main, regardless of enumeration order", async () => {
  for (const windows of [
    [{ handle: "capture", url: CAPTURE }, { handle: "main", url: APP }],
    [{ handle: "main", url: APP }, { handle: "capture", url: CAPTURE }],
  ]) {
    const browser = session(windows, "capture");
    await ensureMainWindow(browser);
    assert.equal(browser.active(), "main");
    assert.equal(browser.switched.length, windows.length + 1, "select once; never retry the journey");
  }
});

test("enumerating an attached main restores that window even when capture is last", async () => {
  const browser = session([{ handle: "main", url: APP }, { handle: "capture", url: CAPTURE }], "main");
  await ensureMainWindow(browser);
  assert.equal(browser.active(), "main");
});

test("a session exposing only capture fails at the window boundary with its URL", async () => {
  const browser = session([{ handle: "capture", url: CAPTURE }], "capture");
  await assert.rejects(ensureMainWindow(browser), /application window.*capture=http:\/\/tauri.localhost\/capture.html/);
});

test("Windows smoke page journeys use the shared navigation and persisted-file doors", () => {
  for (const journey of ["e2e-page-properties.mjs", "e2e-page-trailing-block.mjs"]) {
    const source = fs.readFileSync(path.join(scriptsDir, journey), "utf8");
    assert.ok(source.includes('from "./lib/e2e-navigation.mjs"'),
      `I-9/I-12: ${journey} must use openPageByName from scripts/lib/e2e-navigation.mjs so list readiness repairs reach every journey.`);
    assert.doesNotMatch(source, /async function (?:openPage|gotoPage)\(/,
      `I-9/I-12: ${journey} must not fork page navigation; imitate scripts/e2e-pdf-logseq.mjs.`);
    assert.ok(source.includes('from "./e2e-file-poll.mjs"'),
      `I-9: ${journey} must observe persistence through waitForFileText, not a journey-local timer or poll.`);
    assert.doesNotMatch(source, /const deadline = Date\.now\(\)/,
      `I-9: ${journey} must use the shared waitForFileText observation boundary.`);
  }
});

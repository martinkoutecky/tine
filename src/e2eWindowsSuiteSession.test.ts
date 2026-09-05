// A journey listed in the Windows suite must be able to open a session there.
//
// GH #240 was re-reported on Windows 0.6.98 with no Windows evidence anywhere,
// so `e2e-selection-actions` was added to the `windows-smoke` suite. Listing it
// was not enough: the journey called `tauriCapabilities(APP)` with no
// `debuggerAddress`, which selects EdgeDriver's *launch* mode — the handshake
// `scripts/e2e-capabilities.mjs` documents as unreliable on hosted WebView2
// runners. Run 33959023127 failed in `openSession` with "session not created:
// DevToolsActivePort file doesn't exist", twice, and never executed a single
// product assertion. The run classified itself `infrastructure`, which is
// correct and is exactly the problem: a registered journey that cannot start
// produces a permanently red advisory job carrying no information about the
// bug it was added to catch.
//
// This is I-16 ("a platform choice covers all five shipped targets") one layer
// out from `cfg`: registering behaviour for a platform whose arm nobody wrote.
// On Linux the wry driver launches the app per WebDriver session; on Windows
// the app must be started first with a fixed remote-debugging port and attached
// to. A journey that serves both starts the app through
// `startWebdriverApplication` and forwards `webviewTarget.debuggerAddress` into
// `tauriCapabilities`. The blessed exemplar is
// `scripts/e2e-page-properties.mjs`; `scripts/e2e-og-parity-references.mjs`
// shows the same shape when the journey also has to restart the app.
//
// selection-actions was itself quarantined from the Windows suite the same day
// after two further failures, so it is no longer one of the entries this guard
// checks. The rule it produced still binds every journey that IS listed, and
// the sentinels above are permanent Windows journeys rather than the one that
// prompted the guard - a sentinel naming the motivating case goes stale exactly
// when that case is withdrawn.
import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");

/** The `["name", "scripts/…mjs", {…}]` entries of one suite in run-e2e.mjs. */
export function suiteScripts(source: string, suite: string): Array<{ name: string; script: string }> {
  const opening = source.indexOf(`"${suite}": [`);
  if (opening === -1) throw new Error(`suite ${suite} is not defined in scripts/run-e2e.mjs`);
  const body = source.slice(opening, source.indexOf("\n  ],", opening));
  const entries: Array<{ name: string; script: string }> = [];
  for (const match of body.matchAll(/\[\s*"([^"]+)",\s*"(scripts\/[^"]+)"/g)) {
    entries.push({ name: match[1]!, script: match[2]! });
  }
  return entries;
}

const source = fs.readFileSync(path.join(root, "scripts", "run-e2e.mjs"), "utf8");
const windowsSuite = suiteScripts(source, "windows-smoke");

describe("every windows-smoke journey can open a WebView2 session", () => {
  // The scan is only as good as its input: if the suite table is renamed or
  // reshaped, this test must fail loudly rather than pass over an empty list.
  it("reads a non-empty windows-smoke suite out of run-e2e.mjs", () => {
    expect(windowsSuite.length).toBeGreaterThanOrEqual(8);
    expect(windowsSuite.map((entry) => entry.name)).toContain("windows-core");
    expect(windowsSuite.map((entry) => entry.name)).toContain("page-properties");
  });

  it.each(windowsSuite)("$name starts the app in WebView2 attach mode", ({ name, script }) => {
    const journey = fs.readFileSync(path.join(root, script), "utf8");
    expect(
      journey.includes("startWebdriverApplication"),
      `${script} is registered in the windows-smoke suite but never calls startWebdriverApplication, ` +
        `so on Windows it falls into EdgeDriver launch mode and dies with "DevToolsActivePort file ` +
        `doesn't exist" before any product assertion runs (I-16: a platform choice covers every ` +
        `shipped target). Copy the setup in scripts/e2e-page-properties.mjs — or, if ${name} also ` +
        `restarts the app, scripts/e2e-og-parity-references.mjs.`,
    ).toBe(true);
    expect(
      /tauriCapabilities\([^)]*debuggerAddress/s.test(journey),
      `${script} calls startWebdriverApplication but never forwards webviewTarget.debuggerAddress ` +
        `into tauriCapabilities, so WebDriver still tries to launch the app itself on Windows ` +
        `(I-16). See scripts/e2e-page-properties.mjs.`,
    ).toBe(true);
  });
});

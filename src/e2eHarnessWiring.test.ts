// A native E2E journey that provisions its display AFTER snapshotting
// `process.env` hands the app an environment with no DISPLAY.
//
// `scripts/lib/e2e-display.mjs` exists precisely so a missing display fails
// fast and names the remedy. It cannot help a journey that calls it too late:
// `env` is the object the app is launched with, so building it first and
// calling `ensureDisplay()` afterwards provisions a display the app never
// sees. The app then dies inside the driver with "Failed to initialize gtk
// backend!", and the only thing the journey surfaces is
// `UND_ERR_HEADERS_TIMEOUT` from the WebDriver session POST -- which reads as
// a hang, not as a missing display. That is the exact misdiagnosis
// e2e-display.mjs was written to prevent, and it survived anyway:
// `e2e-pdf-scroll-resources` built `env` at line 81 and called ensureDisplay at
// line 196. It was invisible under `scripts/run-e2e.mjs`, which wraps every
// native Linux scenario in `xvfb-run -a` so DISPLAY is already set, so the
// journey failed only on a direct run and was written off for weeks as
// pre-existing driver-level harness debt.
//
// The blessed exemplar is `scripts/e2e-pdf-ownership.mjs`: `await
// ensureDisplay()` at the top of the file, `const env = {...process.env}`
// afterwards.
//
// The second check keeps `run-e2e.mjs` honest about the environment it
// configures (I-11, the code does not lie about itself): it passed
// `E2E_WINDOW_MANAGER: "openbox"` to `e2e-pdf-scroll-resources`, which never
// read it. A runner that appears to give a journey a window manager, and does
// not, is how `e2e-pdf-logseq` spent 700 lines failing on an X error instead of
// on its missing WM.
import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptsDir = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "scripts");

/**
 * 1-based line of the first `ensureDisplay(...)` call, and of the first
 * `...process.env` snapshot. Either is 0 when absent.
 */
export function displayOrder(source: string): { ensure: number; env: number } {
  const lines = source.split("\n");
  const find = (test: (line: string) => boolean) => {
    const index = lines.findIndex(test);
    return index === -1 ? 0 : index + 1;
  };
  return {
    ensure: find((line) => line.includes("ensureDisplay(") && !line.trimStart().startsWith("import")),
    env: find((line) => /\.\.\.process\.env/.test(line)),
  };
}

/** Journeys `run-e2e.mjs` hands E2E_WINDOW_MANAGER to. */
export function windowManagerScenarios(runner: string): string[] {
  return [...new Set(
    [...runner.matchAll(/"(scripts\/e2e-[^"]+\.mjs)"[^\]]*E2E_WINDOW_MANAGER/g)].map((match) => match[1]!),
  )];
}

const journeys = fs.readdirSync(scriptsDir)
  .filter((name) => name.startsWith("e2e-") && name.endsWith(".mjs"));

describe("native E2E harness wiring", () => {
  it("provisions the display before the environment the app is launched with", () => {
    const late: string[] = [];
    for (const name of journeys) {
      const { ensure, env } = displayOrder(fs.readFileSync(path.join(scriptsDir, name), "utf8"));
      if (ensure === 0 || env === 0) continue;
      if (ensure > env) late.push(`${name}: ensureDisplay at line ${ensure}, ...process.env at line ${env}`);
    }
    expect(
      late,
      "These journeys call ensureDisplay() AFTER snapshotting process.env, so the app is "
        + "launched without DISPLAY and dies with 'Failed to initialize gtk backend!' -- surfacing "
        + "only as UND_ERR_HEADERS_TIMEOUT, which reads as a hang. Move the ensureDisplay() call "
        + "above the `const env = {...process.env}` object. Exemplar: scripts/e2e-pdf-ownership.mjs.",
    ).toEqual([]);
  });

  it("only configures a window manager for journeys that start one", () => {
    const runner = fs.readFileSync(path.join(scriptsDir, "run-e2e.mjs"), "utf8");
    const ignored = windowManagerScenarios(runner).filter((scenario) => {
      const file = path.join(scriptsDir, "..", scenario);
      return fs.existsSync(file) && !fs.readFileSync(file, "utf8").includes("E2E_WINDOW_MANAGER");
    });
    expect(
      ignored,
      "run-e2e.mjs passes E2E_WINDOW_MANAGER to these journeys, but they never read it, so they "
        + "run with no window manager while the runner says otherwise (I-11: the code does not lie "
        + "about itself). Either start the window manager in the journey -- exemplar: "
        + "scripts/e2e-pdf-ownership.mjs, which spawns `process.env.E2E_WINDOW_MANAGER || \"openbox\"` "
        + "-- or drop the vestigial entry from the runner.",
    ).toEqual([]);
  });

  it("both checks fire on known-bad input", () => {
    const bad = ["const env = { ...process.env };", "await ensureDisplay();"].join("\n");
    const order = displayOrder(bad);
    expect(order.env).toBe(1);
    expect(order.ensure).toBe(2);

    const good = ["await ensureDisplay();", "const env = { ...process.env };"].join("\n");
    expect(displayOrder(good).ensure).toBeLessThan(displayOrder(good).env);

    expect(windowManagerScenarios(
      '["some", "scripts/e2e-fake.mjs", { E2E_WINDOW_MANAGER: "openbox" }],',
    )).toEqual(["scripts/e2e-fake.mjs"]);
  });
});

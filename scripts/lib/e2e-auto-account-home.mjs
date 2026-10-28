// Side-effect import for a native journey that is run directly (the integration gate runs
// `node scripts/e2e-X.mjs`, not scripts/run-e2e.mjs). It gives the journey what the suite runner
// gives every scenario: a disposable passwd-account home through the account-home preload shim, so
// the app takes a real kernel owner lease (SB1-OWNER). Without it the app logs
// `owner-lock-unavailable` in a loop and, intermittently, never opens its graph ("h1.page-title still
// not existing after 30000ms", GH #671 journey). Import it FIRST: children inherit process.env.
// Inert under the suite runner (it has already set TINE_E2E_ACCOUNT_SHIM), on --help, and off Linux.
// Opt-in per journey, not blanket (E2E-BASE-4): e2e-desktop-misc fails with the fixture (a click is
// intercepted) and e2e-multigraph ends with "No app-held kernel owner lease observed"; neither is in the
// suite runner's scenario lists, so neither had been run with it. Add it to a journey only after a
// burn-in with and without it.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { installAccountHome } from "./e2e-account-home.mjs";

// `--help` runs launch nothing, so they must neither compile the shim nor demand a lease at exit.
const asksForHelp = process.argv.slice(2).some((arg) => arg === "--help" || arg === "-h");
if (process.platform === "linux" && !process.env.TINE_E2E_ACCOUNT_SHIM && !asksForHelp) {
  const run = fs.mkdtempSync(path.join(process.env.E2E_TMP_ROOT || os.tmpdir(), "tine-e2e-owner-"));
  installAccountHome(run, run);
  // Registered after installAccountHome's own exit hook, so it runs after verify(): keep the
  // evidence (debug logs, owner-lease.json) of a failed run, drop it after a passing one.
  process.once("exit", (code) => {
    if (code === 0 && !process.exitCode) fs.rmSync(run, { recursive: true, force: true });
  });
}

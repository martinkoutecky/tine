// Linux's single-instance transport is the session bus. Every app launch in a
// native journey must share a private bus, including direct script invocations.
// This keeps the journey separate from other tests and from the user's app.
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** The bus configuration every journey's private session runs with. Its listen
 * address is a filesystem socket, because Node cannot connect to an abstract one. */
export const PRIVATE_BUS_CONFIG = path.join(path.dirname(fileURLToPath(import.meta.url)), "e2e-session-bus.conf");

const PRIVATE_BUS = "TINE_E2E_PRIVATE_SESSION_BUS";

/** Command for one isolated Linux journey. The runner supplies DISPLAY first,
 * so D-Bus-activated desktop services inherit the journey's X display too. */
export function privateSessionLaunch(script, args = [], env = process.env) {
  return {
    command: env.DBUS_RUN_SESSION || "dbus-run-session",
    args: ["--config-file", PRIVATE_BUS_CONFIG, "--", process.execPath, script, ...args],
    env: { ...env, [PRIVATE_BUS]: "1" },
  };
}

/** Call before creating fixtures or starting drivers. Direct Linux invocations
 * re-exec once on a private bus; runner-owned private sessions are reused.
 * Windows/macOS use their native single-instance transports and return as-is.
 * Synchronous only in the wrapper process; child output/status are preserved.
 * Missing dbus-run-session is an actionable setup error, never a product timeout.
 */
export function ensurePrivateSessionBus() {
  if (process.platform !== "linux") return;
  if (process.env[PRIVATE_BUS] === "1") {
    if (!process.env.DBUS_SESSION_BUS_ADDRESS) {
      throw new Error("The E2E private session has no DBUS_SESSION_BUS_ADDRESS; launch it with dbus-run-session.");
    }
    return;
  }
  const launch = privateSessionLaunch(process.argv[1], process.argv.slice(2));
  const result = spawnSync(launch.command, launch.args, { env: launch.env, stdio: "inherit" });
  if (result.error) {
    throw new Error("Native launch-forwarding E2E needs dbus-run-session (or DBUS_RUN_SESSION pointing to it).", { cause: result.error });
  }
  process.exit(result.status ?? 1);
}

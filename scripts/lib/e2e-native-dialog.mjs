// Answer Tine's native yes/no confirmation (`backend().confirm`, the GTK
// message dialog of tauri-plugin-dialog) the way a user does: with the
// keyboard, on the dialog window itself. WebDriver cannot see a native dialog,
// so journeys that must pass through a confirmation (rename-merge, journal
// filename renames) use this one helper instead of stubbing `confirm`.
//
// Needs an X display and `xdotool`/`xprop` (run-e2e.mjs puts the portable
// bundle on PATH). The dialog is found by `_NET_WM_WINDOW_TYPE_DIALOG` or a
// `WM_TRANSIENT_FOR` hint among visible windows, so the main window, whose
// title may also be "Tine", is never answered by mistake.
import { execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const XDOTOOL = process.env.E2E_XDOTOOL || "xdotool";

function run(cmd, args, env) {
  try {
    return execFileSync(cmd, args, { encoding: "utf8", env, stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    return "";
  }
}

/** Visible native dialog windows, newest last. Never throws. */
export function nativeDialogWindows(env = process.env) {
  return run(XDOTOOL, ["search", "--onlyvisible", "--name", "."], env)
    .split(/\s+/)
    .filter(Boolean)
    .filter((id) => {
      const props = run("xprop", ["-id", id, "_NET_WM_WINDOW_TYPE", "WM_TRANSIENT_FOR"], env);
      return /_NET_WM_WINDOW_TYPE_DIALOG/.test(props) || /WM_TRANSIENT_FOR\(WINDOW\): window id/.test(props);
    });
}

/** Wait for a native confirmation and answer it. `answer`: "yes" accepts
 *  (Return on the default/affirmative button, then Alt+Y as a fallback),
 *  "no" dismisses with Escape, which GTK reports as a refusal. Resolves once
 *  the dialog is gone; throws naming what it saw otherwise. */
export async function answerNativeDialog(answer, { env = process.env, timeoutMs = 10_000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let id;
  while (!id && Date.now() < deadline) {
    id = nativeDialogWindows(env).at(-1);
    if (!id) await sleep(100);
  }
  if (!id) throw new Error("no native confirmation dialog appeared");
  const title = run(XDOTOOL, ["getwindowname", id], env);
  const keys = answer === "yes" ? ["alt+y", "Return"] : ["Escape"];
  for (const key of keys) {
    if (!nativeDialogWindows(env).includes(id)) break;
    run(XDOTOOL, ["windowfocus", "--sync", id], env);
    run(XDOTOOL, ["key", "--clearmodifiers", key], env);
    const gone = Date.now() + 2_000;
    while (Date.now() < gone && nativeDialogWindows(env).includes(id)) await sleep(100);
  }
  if (nativeDialogWindows(env).includes(id)) {
    throw new Error(`native dialog ${JSON.stringify(title)} did not close after answering ${answer}`);
  }
  return { title };
}

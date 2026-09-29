// "A newer Tine is available" check — best-effort, once per launch.
//
// Notifier: ask the Tauri updater plugin what the og-preview channel offers (NEVER
// the shipped Tine's `releases/latest` — see RELEASES_PAGE) and, if it's newer than
// the running build, show a sticky toast. This is the cross-platform half and is
// always the way a user LEARNS an update exists.
//
// Installer (the toast's action): on **Windows/Linux** in the packaged app, run the
// Tauri v2 updater — `check()` → `downloadAndInstall()` → `relaunch()` — so the
// update applies in place. On **macOS** (bundle is unsigned → Gatekeeper would
// reject a self-replaced app) and outside Tauri, fall back to opening the releases
// page in the browser. Android/iOS update through their distribution channel, so
// both the notifier and installer are disabled there. The updater is inert until
// a signed release with a `latest.json` exists; any failure (no manifest yet, bad
// signature, offline) is caught and also falls back to the releases page — it can
// never brick the app.
//
// Deliberately quiet: Tauri-only check, silent on ANY failure (offline, rate-
// limited, blocked) — it must never block startup or nag with an error. A failed
// INSTALL the user asked for is different: it records a fixed stage/cause in the
// privacy-safe diagnostic report and says which stage failed (GH #343).

import { isTauri, backend } from "./backend";
import { dbg, recordDiagnostic } from "./debug";
import { ownedWhen, readOwned } from "./owned";
import { platformKind } from "./platform";
import { pushToast, dismissToast } from "./toasts";
import { openSettings } from "./ui";
import { reportUiFailure } from "./uiFailure";

/** THE update channel (og-only). This build (`page.tine.TineOG`) must never offer
 * the shipped Tine: `releases/latest` there carries a higher version number, and
 * installing it would REPLACE og with master. The channel is the fixed-tag GitHub
 * release `og-preview`. What it offers is answered ONCE, by the Tauri updater
 * plugin: `check()` reads the endpoint in `tauri.conf.json` (Rust-side, so no
 * webview CORS problem: GitHub release-asset downloads send no
 * Access-Control-Allow-Origin) and the installer downloads from that same
 * manifest. This file therefore names no channel URL to fetch and never calls
 * `fetch()`; the only URL here is the human-facing release page below (guard:
 * `src/updateChannel.guard.test.ts`). */
const RELEASES_PAGE = "https://github.com/martinkoutecky/tine/releases/tag/og-preview";

/** Parse the first `X.Y.Z` out of a version/tag string (`v0.3.0`, `0.3.0`, …). */
function parseVer(s: string): [number, number, number] | null {
  const m = /(\d+)\.(\d+)\.(\d+)/.exec(s);
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

type UpdateMode = "self" | "manual" | "unavailable";

/** Resolve update behavior conservatively. Mobile builds update through their
 * distribution channel; a platform-detection failure must therefore fail closed
 * instead of accidentally exposing the desktop updater. */
async function updateMode(): Promise<UpdateMode> {
  if (!isTauri()) return "unavailable";
  try {
    if ((await platformKind()) !== "desktop") return "unavailable";
  } catch {
    return "unavailable";
  }
  return /\bMac/i.test(typeof navigator !== "undefined" ? navigator.userAgent : "")
    ? "manual"
    : "self";
}

export type UpdaterFailureStage =
  | "manifest_fetch" | "manifest_parse" | "target_selection" | "download"
  | "signature_verification" | "install" | "relaunch";
export type UpdaterFailureCause =
  | "network" | "invalid_manifest" | "unsupported_target" | "invalid_signature"
  | "install_failed" | "relaunch_failed" | "unknown";
type UpdaterFailurePhase = "check" | "apply" | "relaunch";
type UpdaterFailure = { stage: UpdaterFailureStage; cause: UpdaterFailureCause };

function errorText(error: unknown): string {
  const parts: string[] = [];
  const seen = new Set<object>();
  let current: unknown = error;
  while (current != null && parts.length < 4) {
    if (typeof current === "string") {
      parts.push(current);
      break;
    }
    if (typeof current !== "object" || seen.has(current)) break;
    seen.add(current);
    const said = (current as { message?: unknown }).message;
    if (typeof said === "string" && said.trim()) parts.push(said);
    current = (current as { cause?: unknown }).cause;
  }
  return parts.join(" caused by ") || "unknown updater failure";
}

/** Reduce an updater plugin failure to fixed stage/cause tokens before it
 * enters the always-on diagnostic report; the raw text never does. `phase` is
 * the call that failed (`check()`, `downloadAndInstall()`, `relaunch()`).
 * Pure; O(length of the error's message chain, at most four links). */
export function classifyUpdaterFailure(phase: UpdaterFailurePhase, error: unknown): UpdaterFailure {
  const text = errorText(error).toLowerCase();
  if (phase === "relaunch") return { stage: "relaunch", cause: "relaunch_failed" };
  if (phase === "check") {
    if (/request|network|connect|connection|dns|tls|certificate|proxy|timed? ?out/.test(text)) {
      return { stage: "manifest_fetch", cause: "network" };
    }
    if (/none of the fallback platforms (?:was|were) found|platform .+ (?:was|is) not found in (?:the )?(?:release|response)/.test(text)) {
      return { stage: "target_selection", cause: "unsupported_target" };
    }
    if (/invalid json|deserialize|failed to parse|release response|release manifest|missing field|unknown field|invalid type|expected (?:value|identifier|struct|sequence|map)|trailing characters|eof while parsing|key must be a string/.test(text)) {
      return { stage: "manifest_parse", cause: "invalid_manifest" };
    }
    return { stage: "manifest_fetch", cause: "unknown" };
  }
  if (/minisign|signature|base64/.test(text)) return { stage: "signature_verification", cause: "invalid_signature" };
  if (/request|network|download|connect|connection|dns|tls|certificate|proxy|timed? ?out/.test(text)) {
    return { stage: "download", cause: "network" };
  }
  if (/install|package|authentication|permission|access denied|binary|archive|rename/.test(text)) {
    return { stage: "install", cause: "install_failed" };
  }
  // `downloadAndInstall` is one upstream call; with no classifiable cause, do
  // not pretend to know which internal sub-step failed.
  return { stage: "install", cause: "unknown" };
}

/** The error chain with URLs, credentials and paths replaced, bounded to 800
 * characters — for the opt-in debug log only. Pure. */
export function safeUpdaterErrorChain(error: unknown): string {
  return errorText(error)
    .replace(/\b(?:https?|file):\/\/[^\s"'<>]+/gi, "<url>")
    .replace(/\b(proxy[-_ ]?authorization|authorization)\s*[:=]\s*[^\r\n,;]+/gi, "$1=<redacted>")
    .replace(/\b(password|passwd|access[_-]?token|api[_-]?key|token|credential|secret)\s*[:=]\s*(?:"[^"]*"|'[^']*'|[^\s,;]+)/gi, "$1=<redacted>")
    .replace(/(?:[A-Za-z]:[\\/]|\\\\|\/\/)[^\r\n,;]+/g, "<path>")
    .replace(/(^|\s|\(|"|')\/(?:home|Users|tmp|var|etc)\/[^\r\n,;)\]"']+/gi, "$1<path>")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 800);
}

const STAGE_LABEL: Record<UpdaterFailureStage, string> = {
  manifest_fetch: "manifest fetch",
  manifest_parse: "manifest parsing",
  target_selection: "platform selection",
  download: "download",
  signature_verification: "signature verification",
  install: "installation",
  relaunch: "relaunch",
};

/** Record a failed update step (fixed tokens only; the scrubbed chain goes to
 * the opt-in log) and tell the user which stage failed, with a way to the
 * Diagnostics report. */
function reportUpdaterFailure(phase: UpdaterFailurePhase, error: unknown): void {
  const failure = classifyUpdaterFailure(phase, error);
  void recordDiagnostic("updater_failure", { updaterStage: failure.stage, updaterCause: failure.cause });
  dbg(`updater failure stage=${failure.stage} cause=${failure.cause}: ${safeUpdaterErrorChain(error)}`);
  const what = phase === "relaunch"
    ? "The update installed but Tine could not relaunch."
    : `Couldn't apply the update during ${STAGE_LABEL[failure.stage]} — opening the releases page instead.`;
  pushToast(`${what} Diagnostics has the safe failure stage; launch Tine with --debug for the sanitized cause chain.`, "error", {
    action: { label: "Diagnostics", run: () => openSettings("diagnostics") },
  });
}

/** Whether this binary is the experimental 32-bit Windows build, which updates
 * manually by policy (GH #594). An unanswerable probe counts as "no". */
async function isManualOnlyBuild(): Promise<boolean> {
  try {
    const architecture = await readOwned(ownedWhen(), backend().appArchitecture());
    return architecture.kind === "current" && architecture.value === "x86";
  } catch (error) {
    // The packaged app always has this command; a browser/mock boundary keeps
    // the established offer.
    dbg(`update architecture probe failed: ${String(error)}`);
    return false;
  }
}

/** Offer a manual download on a manual-only build. Recorded as its own
 * `updater.manual_only` event, never as an `updater.failure` (GH #594). */
function offerManualOnly(version: string): number {
  void recordDiagnostic("updater_manual_only");
  return pushToast(
    `Tine ${version} is available, but automatic updates are not supported by this experimental 32-bit Windows build. Download the x86 package manually.`,
    "warn",
    { sticky: true, action: { label: "Download manually", run: openReleases } },
  );
}

/** Open the GitHub releases page in the system browser (the manual fallback). */
function openReleases(): void {
  void backend().openExternal(RELEASES_PAGE).catch((error) => reportUiFailure("external-link", error));
}

/** The window's one persistence-before-exit transaction (App's `safeClose`),
 * injected because App imports this module. Installing an update ends the
 * process (the Windows installer exits it; Linux relaunches), so it must pass
 * through the same flush-and-confirm gate as closing the window (I-2, I-12). */
export interface UpdateExitGuard {
  prepare(): Promise<"accepted" | "rejected" | "in_flight">;
  reset(): void;
}
let exitGuard: UpdateExitGuard | null = null;
export function setUpdateExitGuard(guard: UpdateExitGuard | null): void {
  exitGuard = guard;
}

/** The toast's "Install update" action. Win/Linux packaged app → run the Tauri updater
 *  in place and relaunch; everything else (macOS, browser, or any failure) → open
 *  the releases page. Never throws. */
async function applyUpdateOrOpen(): Promise<void> {
  const mode = await updateMode();
  if (mode === "unavailable") return;
  if (mode === "manual") {
    openReleases();
    return;
  }
  let update: Awaited<ReturnType<(typeof import("@tauri-apps/plugin-updater"))["check"]>>;
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    update = await check();
  } catch (error) {
    reportUpdaterFailure("check", error);
    openReleases();
    return;
  }
  if (!update) {
    // No signed `latest.json` yet (or already current) → manual path.
    openReleases();
    return;
  }
  const progressId = pushToast(`Downloading Tine ${update.version}…`, "info", { sticky: true });
  try {
    // Download first: the user keeps editing meanwhile, so the flush below sees
    // the latest state and runs immediately before the process can exit.
    await update.download();
  } catch (error) {
    dismissToast(progressId);
    reportUpdaterFailure("apply", error);
    openReleases(); // signature/verify/network failure → never brick, just offer the page
    return;
  }
  // Every in-flight save and at-risk draft is flushed (bounded), and a failed
  // flush asks the user before anything is discarded: the window-close gate.
  let gate: "accepted" | "rejected" | "in_flight" = "rejected";
  try { gate = exitGuard ? await exitGuard.prepare() : "rejected"; }
  catch (error) { dbg(`update exit guard failed: ${String(error)}`); }
  if (gate !== "accepted") {
    dismissToast(progressId);
    pushToast(
      "The update was downloaded but not installed, so your unsaved changes stay open. Choose Install update again when you are ready.",
      "warn",
    );
    return;
  }
  try {
    await update.install();
  } catch (error) {
    exitGuard?.reset();
    dismissToast(progressId);
    reportUpdaterFailure("apply", error);
    openReleases();
    return;
  }
  try {
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch(); // process restarts into the new version (this toast goes with it)
  } catch (error) {
    exitGuard?.reset();
    dismissToast(progressId);
    reportUpdaterFailure("relaunch", error);
  }
}

/** The one visible offer: startup and the About tab's explicit check can find
 * the same release, and a second sticky toast must replace the first. */
let offeredUpdateToastId: number | null = null;
let offerGeneration = 0;

/** Publish the sticky "update available" toast. Checking never installs by
 * itself: the user picks "Install update" (or "Download manually" on the
 * manual-only build). Startup and an explicit check may resolve concurrently;
 * only the newest attempt replaces the singleton toast (master 5cc573f2,
 * b80c54f3, ca1b48f5). O(1) plus one architecture probe.
 * @internal Exported for deterministic concurrency coverage. */
export async function offerUpdate(version: string, current: string): Promise<void> {
  const generation = ++offerGeneration;
  const manualOnly = await isManualOnlyBuild();
  if (generation !== offerGeneration) return;
  if (offeredUpdateToastId !== null) dismissToast(offeredUpdateToastId);
  if (manualOnly) {
    offeredUpdateToastId = offerManualOnly(version);
    return;
  }
  offeredUpdateToastId = pushToast(
    `Tine ${version} is available — you're on ${current}.`,
    "info",
    {
      sticky: true,
      action: { label: "Install update", run: () => void applyUpdateOrOpen() },
    },
  );
}

/** The version the og-preview channel offers when it is newer than this build,
 *  else null. The updater plugin decides "newer" (and reads the manifest); it
 *  throws on a missing, unreachable or invalid manifest, which callers absorb.
 *  Releases the plugin's resource handle (the installer takes its own). */
async function offeredVersion(): Promise<[number, number, number] | null> {
  const { check } = await import("@tauri-apps/plugin-updater");
  const offer = await check();
  if (!offer) return null;
  const version = parseVer(offer.version);
  try { await offer.close(); } catch (error) { dbg(`updater handle close failed: ${String(error)}`); }
  return version;
}

/** Check the og-preview channel for a newer build; toast if there is one.
 *  Resolves silently (never throws) in every failure case. */
export async function checkForUpdate(): Promise<void> {
  if ((await updateMode()) === "unavailable") return;
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    const cur = parseVer(await getVersion());
    if (!cur) return;
    const latest = await offeredVersion();
    if (!latest) return;
    await offerUpdate(latest.join("."), cur.join("."));
  } catch {
    // offline / rate-limited / network blocked — never bother the user.
  }
}

export type UpdateStatus =
  | { kind: "current"; version: string }
  | { kind: "available"; version: string; current: string }
  | { kind: "unavailable" }; // offline, rate-limited, no preview release, or not the packaged app

/** The About tab's explicit "Check for updates" button. Unlike `checkForUpdate`
 *  (silent on the common no-update path), this reports every outcome so the
 *  button can show feedback. Checking never installs by itself: an available
 *  build gets an explicit Install update action in a sticky toast. Never throws. */
export async function checkForUpdateNow(): Promise<UpdateStatus> {
  if ((await updateMode()) === "unavailable") return { kind: "unavailable" };
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    const cur = parseVer(await getVersion());
    if (!cur) return { kind: "unavailable" };
    const offered = await offeredVersion();
    if (!offered) return { kind: "current", version: cur.join(".") };
    const version = offered.join(".");
    const current = cur.join(".");
    await offerUpdate(version, current);
    return { kind: "available", version, current };
  } catch {
    return { kind: "unavailable" };
  }
}

/** Open the og-preview releases page (exported for the About tab's manual link). */
export function openReleasesPage(): void {
  openReleases();
}

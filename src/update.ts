// "A newer Tine is available" check — best-effort, once per launch.
//
// Notifier: ask GitHub for the latest *published* release and, if it's newer than
// the running build, show a sticky toast. This is the cross-platform half and is
// always the way a user LEARNS an update exists.
//
// Installer (the toast's action): on **Windows/Linux** in the packaged app, run the
// Tauri v2 updater — `check()` → `downloadAndInstall()` → `relaunch()` — so the
// update applies in place. On **macOS** (bundle is unsigned → Gatekeeper would
// reject a self-replaced app) and outside Tauri, fall back to opening the releases
// page in the browser. The updater is inert until a signed release with a
// `latest.json` exists; any failure (no manifest yet, bad signature, offline) is
// caught and also falls back to the releases page — it can never brick the app.
//
// Deliberately quiet: Tauri-only check, silent on ANY failure (offline, rate-
// limited, blocked) — it must never block startup or nag with an error.

import { isTauri, backend } from "./backend";
import { pushToast, dismissToast } from "./ui";

const REPO = "martinkoutecky/tine";
const RELEASES_PAGE = `https://github.com/${REPO}/releases/latest`;
const LATEST_API = `https://api.github.com/repos/${REPO}/releases/latest`;

/** Parse the first `X.Y.Z` out of a version/tag string (`v0.3.0`, `0.3.0`, …). */
function parseVer(s: string): [number, number, number] | null {
  const m = /(\d+)\.(\d+)\.(\d+)/.exec(s);
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

/** Is `a` a strictly newer semver triple than `b`? */
function isNewer(a: [number, number, number], b: [number, number, number]): boolean {
  for (let i = 0; i < 3; i++) {
    if (a[i] !== b[i]) return a[i] > b[i];
  }
  return false;
}

/** True only where an in-place self-update is safe: the packaged Tauri app on a
 *  non-macOS OS. (macOS bundles are unsigned; replacing one re-triggers Gatekeeper
 *  quarantine, so those get the manual download path instead.) */
function canSelfUpdate(): boolean {
  return isTauri() && !/\bMac/i.test(typeof navigator !== "undefined" ? navigator.userAgent : "");
}

/** Open the GitHub releases page in the system browser (the manual fallback). */
function openReleases(): void {
  void backend().openExternal(RELEASES_PAGE).catch(() => {});
}

/** The toast's "Download" action. Win/Linux packaged app → run the Tauri updater
 *  in place and relaunch; everything else (macOS, browser, or any failure) → open
 *  the releases page. Never throws. */
async function applyUpdateOrOpen(): Promise<void> {
  if (!canSelfUpdate()) {
    openReleases();
    return;
  }
  let progressId: number | null = null;
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) {
      // No signed `latest.json` yet (or already current) → manual path.
      openReleases();
      return;
    }
    progressId = pushToast(`Downloading Tine ${update.version}…`, "info", { sticky: true });
    await update.downloadAndInstall();
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch(); // process restarts into the new version (this toast goes with it)
  } catch {
    if (progressId != null) dismissToast(progressId);
    openReleases(); // signature/verify/network failure → never brick, just offer the page
  }
}

/** Check GitHub for a newer published release; toast if there is one. Resolves
 *  silently (never throws) in every failure case. */
export async function checkForUpdate(): Promise<void> {
  if (!isTauri()) return;
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    const cur = parseVer(await getVersion());
    if (!cur) return;

    // `/releases/latest` is the newest NON-prerelease, NON-draft release.
    const res = await fetch(LATEST_API, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) return;
    const data: unknown = await res.json();
    const tag = (data as { tag_name?: unknown })?.tag_name;
    const latest = typeof tag === "string" ? parseVer(tag) : null;
    if (!latest || !isNewer(latest, cur)) return;

    pushToast(
      `Tine ${latest.join(".")} is available — you're on ${cur.join(".")}.`,
      "info",
      {
        sticky: true,
        action: {
          label: "Download",
          run: () => void applyUpdateOrOpen(),
        },
      }
    );
  } catch {
    // offline / rate-limited / network blocked — never bother the user.
  }
}

export type UpdateStatus =
  | { kind: "current"; version: string }
  | { kind: "available"; version: string; current: string }
  | { kind: "unavailable" }; // offline, rate-limited, or not the packaged app

/** The About tab's explicit "Check for updates" button. Unlike `checkForUpdate`
 *  (silent on the common no-update path), this reports every outcome so the
 *  button can show feedback. If a newer release exists, kicks off the same
 *  download-or-open flow as the startup toast. Never throws. */
export async function checkForUpdateNow(): Promise<UpdateStatus> {
  if (!isTauri()) return { kind: "unavailable" };
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    const curStr = await getVersion();
    const cur = parseVer(curStr);
    if (!cur) return { kind: "unavailable" };

    const res = await fetch(LATEST_API, { headers: { Accept: "application/vnd.github+json" } });
    if (!res.ok) return { kind: "unavailable" };
    const data: unknown = await res.json();
    const tag = (data as { tag_name?: unknown })?.tag_name;
    const latest = typeof tag === "string" ? parseVer(tag) : null;
    if (!latest) return { kind: "unavailable" };

    if (isNewer(latest, cur)) {
      void applyUpdateOrOpen();
      return { kind: "available", version: latest.join("."), current: cur.join(".") };
    }
    return { kind: "current", version: cur.join(".") };
  } catch {
    return { kind: "unavailable" };
  }
}

/** Open the GitHub releases page (exported for the About tab's manual link). */
export function openReleasesPage(): void {
  openReleases();
}

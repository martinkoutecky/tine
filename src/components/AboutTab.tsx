// The "About" settings tab (GH #32): version, build info, project links, and
// credits. Read-only — it configures nothing; it lives in Settings only because
// that's already where Tine keeps its other informational panes (shortcuts,
// backups, help-improve) and it needs no separate window plumbing.
import { createSignal, onMount, Show, type JSX } from "solid-js";
import { backend, isTauri } from "../backend";
import { platformKind } from "../platform";
import { checkForUpdateNow, openReleasesPage } from "../update";

const WEBSITE = "https://tine.page";
const REPO = "https://github.com/martinkoutecky/tine";
const ISSUES = "https://github.com/martinkoutecky/tine/issues";
const CHANGELOG = "https://github.com/martinkoutecky/tine/blob/HEAD/CHANGELOG.md";
const KOFI = "https://ko-fi.com/martinkoutecky";

function openExternal(url: string) {
  void backend().openExternal(url).catch(() => {});
}

// Build-time constants (vite.config.ts). __GIT_COMMIT__ is "" outside a git
// checkout — the commit row is hidden then.
function buildStamp(): string {
  try {
    return new Date(__BUILD_TIME__).toLocaleString();
  } catch {
    return __BUILD_TIME__;
  }
}

export function AboutTab(): JSX.Element {
  const [version, setVersion] = createSignal("");
  const [status, setStatus] = createSignal("");
  const [checking, setChecking] = createSignal(false);
  const [updatePlatform, setUpdatePlatform] = createSignal<"loading" | "desktop" | "mobile" | "unavailable">(
    isTauri() ? "loading" : "unavailable"
  );

  onMount(async () => {
    if (!isTauri()) return;
    try {
      setUpdatePlatform((await platformKind()) === "desktop" ? "desktop" : "mobile");
    } catch {
      // Fail closed: an unknown native platform must not expose the desktop updater.
      setUpdatePlatform("unavailable");
    }
    try {
      const { getVersion } = await import("@tauri-apps/api/app");
      setVersion(await getVersion());
    } catch {
      /* dev / non-Tauri — no runtime version */
    }
  });

  const check = async () => {
    setChecking(true);
    setStatus("");
    const r = await checkForUpdateNow();
    setChecking(false);
    if (r.kind === "current") setStatus(`You're on the latest version (${r.version}).`);
    else if (r.kind === "available") setStatus(`Tine ${r.version} is available — downloading…`);
    else setStatus("Couldn't check right now — see the releases page.");
  };

  return (
    <div class="about-tab">
      <div class="about-head">
        <svg class="about-mark" width="44" height="55" viewBox="0 0 64 80" aria-hidden="true">
          <g stroke="currentColor" stroke-width="6" stroke-linecap="round" fill="none">
            <path d="M16 14 V36" /><path d="M32 14 V36" /><path d="M48 14 V36" />
            <path d="M16 36 H48" /><path d="M32 36 V70" />
          </g>
        </svg>
        <div class="about-title">
          <div class="about-name">Tine</div>
          <div class="about-tagline">A fast, local-first, Logseq-compatible outliner.</div>
        </div>
      </div>

      <div class="about-version">
        <Show when={version()} fallback={<span class="settings-hint">Development build</span>}>
          <span class="about-ver-num">Version {version()}</span>
        </Show>
        <Show when={__GIT_COMMIT__}>
          <span class="about-commit mono">· {__GIT_COMMIT__}</span>
        </Show>
        <Show when={updatePlatform() === "desktop"}>
          <button class="btn-secondary about-check" onClick={check} disabled={checking()}>
            {checking() ? "Checking…" : "Check for updates"}
          </button>
        </Show>
      </div>
      <Show when={updatePlatform() === "mobile"}>
        <div class="settings-hint about-status">
          Updates arrive through your app's distribution channel.
        </div>
      </Show>
      <Show when={status()}>
        <div class="settings-hint about-status">
          {status()}{" "}
          <button class="about-linkbtn" onClick={() => openReleasesPage()}>Releases</button>
        </div>
      </Show>
      <div class="settings-hint about-build">Built {buildStamp()}</div>

      <div class="about-links">
        <button class="about-link" onClick={() => openExternal(WEBSITE)}>
          <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.7">
            <circle cx="12" cy="12" r="9" /><path d="M3 12h18" />
            <path d="M12 3a15 15 0 0 1 0 18a15 15 0 0 1 0-18z" />
          </svg>
          <span>Website</span>
          <span class="about-link-url">tine.page</span>
        </button>

        <button class="about-link" onClick={() => openExternal(REPO)}>
          <svg viewBox="0 0 24 24" aria-hidden="true" fill="currentColor">
            <path d="M12 1.5A10.5 10.5 0 0 0 8.68 22a.55.55 0 0 0 .73-.53c0-.26-.01-1.13-.02-2.05-2.92.64-3.54-1.24-3.54-1.24-.48-1.22-1.17-1.54-1.17-1.54-.95-.65.07-.64.07-.64 1.06.07 1.62 1.09 1.62 1.09.94 1.61 2.47 1.14 3.07.87.1-.68.37-1.14.66-1.4-2.33-.27-4.78-1.17-4.78-5.18 0-1.15.41-2.08 1.08-2.82-.11-.27-.47-1.35.1-2.8 0 0 .88-.28 2.88 1.07a10 10 0 0 1 5.24 0c2-1.35 2.88-1.07 2.88-1.07.57 1.45.21 2.53.1 2.8.68.74 1.08 1.67 1.08 2.82 0 4.02-2.46 4.9-4.8 5.16.38.33.71.97.71 1.96 0 1.42-.01 2.56-.01 2.91 0 .28.19.61.74.51A10.5 10.5 0 0 0 12 1.5z" />
          </svg>
          <span>Source code</span>
          <span class="about-link-url">GitHub</span>
        </button>

        <button class="about-link" onClick={() => openExternal(KOFI)}>
          <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round">
            <path d="M4 8h13v5a4 4 0 0 1-4 4H8a4 4 0 0 1-4-4z" />
            <path d="M17 9h1.8a2.2 2.2 0 0 1 0 4.4H17" />
            <path d="M8 3.2c-.6.7-.6 1.4 0 2.1M11.5 3.2c-.6.7-.6 1.4 0 2.1" stroke-linecap="round" />
          </svg>
          <span>Support Tine</span>
          <span class="about-link-url">Ko-fi</span>
        </button>
      </div>

      <div class="about-meta">
        <button class="about-linkbtn" onClick={() => openExternal(CHANGELOG)}>Changelog</button>
        <span class="about-dot">·</span>
        <button class="about-linkbtn" onClick={() => openExternal(ISSUES)}>Report an issue</button>
        <span class="about-dot">·</span>
        <span>License: AGPL-3.0-only</span>
      </div>

      <div class="about-credits">
        <div class="about-credit-row">
          <span class="about-credit-who">Martin Koutecký</span>
          <span class="about-credit-what">direction, design, and authorship</span>
        </div>
        <div class="about-credit-row">
          <span class="about-credit-who">Claude Code &amp; Codex</span>
          <span class="about-credit-what">engineering and analysis</span>
        </div>
      </div>
    </div>
  );
}

import { Rich } from "../i18n/Rich";
import { msg, t } from "../i18n";
import { createEffect, createResource, createSignal, onCleanup, Show, type JSX } from "solid-js";
import { switchGraph, createNewGraph, loadGraphPath } from "../graph";
import { graphMeta, startupOpenFailure, setStartupOpenFailure } from "../graphSession";
import { writeClipboardText } from "../clipboard";
import { pushToast } from "../toasts";
import { isTauri } from "../backend";
import { WindowControls } from "./WindowChrome";
import { osDrawsWindowControls } from "../nativeChrome";
import { registerTransientLayer } from "../transientLayers";
import { queryAllWindows } from "../windowRealm";
import { platformKind } from "../platform";
import { readOr } from "../resourceRead";

/** First-run onboarding. Shown (as a full-cover layer) when the app starts with
 *  no graph configured: choose to open an existing Logseq graph, or create a new
 *  one that comes pre-loaded with a short guided demo. */
export function Welcome(props: { onClose?: () => void } = {}): JSX.Element {
  const [busy, setBusy] = createSignal<null | "open" | "create">(null);
  // iOS opens graphs only inside the app's own containers (master 027b5ae15).
  const [platform] = createResource(platformKind);
  const ios = () => readOr(platform, undefined, "welcome-platform") === "ios";

  const run = (which: "open" | "create", fn: () => Promise<unknown>) => async () => {
    if (busy()) return;
    setBusy(which);
    try {
      await fn();
      props.onClose?.();
    } finally {
      // If the graph loaded, this layer unmounts anyway (graphMeta is set); if the
      // picker was cancelled, re-enable the buttons.
      setBusy(null);
    }
  };

  // A graph chosen at launch that would not open: say which one and why, and
  // offer the way out (retry it, pick another, copy the details) instead of a
  // silent first-run screen that looks as if the graph were never configured.
  const failure = () => (graphMeta() ? null : startupOpenFailure());
  const retry = run("open", async () => {
    const failed = startupOpenFailure();
    if (!failed) return;
    try {
      await loadGraphPath(failed.path);
      setStartupOpenFailure(null);
    } catch (e) {
      setStartupOpenFailure({ path: failed.path, message: String(e) });
    }
  });
  const copyDetails = () => {
    const failed = failure();
    if (!failed) return;
    void writeClipboardText(`Could not open ${failed.path}\n${failed.message}`)
      .catch((e) => pushToast(msg("welcome-could-not-copy", { error: String(e) }), "error"));
  };

  return (
    <div class="welcome-overlay">
      {/* Frameless main window (decorations:false) → the topbar's window controls
          are covered by this full-cover overlay, leaving no way to close the app on
          first run (GH #36). Draw them here (drag strip + min/max/close) whenever the
          OS isn't already drawing a frame. macOS/native-frame/mobile skip this. */}
      <Show when={isTauri() && !osDrawsWindowControls()}>
        <div class="welcome-winchrome" data-tauri-drag-region>
          <WindowControls />
        </div>
      </Show>
      <div class="welcome-card">
        <Show when={props.onClose}>
          <button class="welcome-close" title={t("welcome-close-welcome")} onClick={() => props.onClose?.()}>
            x
          </button>
        </Show>
        <svg class="welcome-mark" width="52" height="64" viewBox="0 0 64 80" aria-hidden="true">
          <g stroke="currentColor" stroke-width="6" stroke-linecap="round" fill="none">
            <path d="M16 14 V36" /><path d="M32 14 V36" /><path d="M48 14 V36" />
            <path d="M16 36 H48" /><path d="M32 36 V70" />
          </g>
        </svg><h1 class="welcome-title">{t("welcome-welcome-to-tine")}</h1>
        <p class="welcome-lede"><Rich message={msg("welcome-a-fast-local-outliner-for-your-logseq")} slots={{ "strong": (children) => <strong>{children}</strong> }} /></p>

        <Show when={failure()}>
          {(failed) => (
            <div class="welcome-recovery" role="alert"><p class="welcome-recovery-title">{t("welcome-tine-could-not-open-your-last-graph")}</p>
              <p class="welcome-recovery-path">{failed().path}</p>
              <p class="welcome-recovery-reason">{failed().message}</p>
              <div class="welcome-recovery-actions"><button class="settings-btn" type="button" disabled={!!busy()} onClick={retry}>{t("welcome-try-again")}</button><button class="settings-btn" type="button" onClick={copyDetails}>{t("welcome-copy-details")}</button>
              </div><p class="welcome-recovery-note">{t("welcome-nothing-was-changed-on-disk-you-can")}</p>
            </div>
          )}
        </Show>

        <div class="welcome-actions">
          <Show when={ios()}>
            <p class="welcome-busy">{t("welcome-on-iphone-and-ipad-graphs-can-live")}</p>
          </Show>
          <button
            class="welcome-choice"
            disabled={!!busy()}
            onClick={run("open", switchGraph)}
          >
            <svg class="welcome-choice-ic" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M3 7a2 2 0 0 1 2-2h3.6l2 2H19a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"
                fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round" />
            </svg>
            <span class="welcome-choice-text"><span class="welcome-choice-title">{t("welcome-open-an-existing-graph")}</span><span class="welcome-choice-sub">{t("welcome-point-tine-at-a-logseq-graph-folder")}</span>
            </span>
          </button>

          <button
            class="welcome-choice welcome-choice-primary"
            disabled={!!busy()}
            onClick={run("create", createNewGraph)}
          >
            <svg class="welcome-choice-ic" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M12 3.5l1.9 5.6 5.6 1.9-5.6 1.9L12 18.5l-1.9-5.6L4.5 11l5.6-1.9z"
                fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" />
            </svg>
            <span class="welcome-choice-text"><span class="welcome-choice-title">{t("welcome-create-a-new-graph")}</span>
              <span class="welcome-choice-sub">{t("welcome-new-to-this-we-ll-set-up-a")}</span>
            </span>
          </button>
        </div>

        <Show when={busy() === "create"}><p class="welcome-busy">{t("welcome-setting-up-your-graph")}</p>
        </Show>
        <Show when={busy() === "open"}><p class="welcome-busy">{t("welcome-opening")}</p>
        </Show>

        <p class="welcome-foot">{t("welcome-free-open-source-your-notes")}</p>
      </div>
    </div>
  );
}

/** Production truth-table seam: first-load/forced Welcome is mandatory even
 * when Help's optional signal is also set.  Only the optional-on-a-graph layer
 * owns a transient dismissal rung. */
export function WelcomeLayer(props: {
  mandatory: boolean;
  optionalOpen: boolean;
  onClose: () => void;
}): JSX.Element {
  const visible = () => props.mandatory || props.optionalOpen;
  const dismissible = () => props.optionalOpen && !props.mandatory;
  // First-load Welcome has no close action and deliberately does not consume
  // Escape/Back. A Help-opened Welcome over a real graph is the distinct owner.
  createEffect(() => {
    if (!dismissible()) return;
    const unregister = registerTransientLayer({
      id: "welcome",
      root: () => queryAllWindows<HTMLElement>(".welcome-card"),
      dismiss: () => { props.onClose(); return true; },
    });
    onCleanup(unregister);
  });
  return (
    <Show when={visible()}>
      <Welcome onClose={dismissible() ? props.onClose : undefined} />
    </Show>
  );
}

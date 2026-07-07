import { createSignal, Show, type JSX } from "solid-js";
import { switchGraph, createNewGraph } from "../graph";
import { isTauri } from "../backend";
import { WindowControls } from "./WindowChrome";
import { osDrawsWindowControls } from "../nativeChrome";

/** First-run onboarding. Shown (as a full-cover layer) when the app starts with
 *  no graph configured: choose to open an existing Logseq graph, or create a new
 *  one that comes pre-loaded with a short guided demo. */
export function Welcome(props: { onClose?: () => void } = {}): JSX.Element {
  const [busy, setBusy] = createSignal<null | "open" | "create">(null);

  const run = (which: "open" | "create", fn: () => Promise<void>) => async () => {
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
          <button class="welcome-close" title="Close welcome" onClick={() => props.onClose?.()}>
            x
          </button>
        </Show>
        <svg class="welcome-mark" width="52" height="64" viewBox="0 0 64 80" aria-hidden="true">
          <g stroke="currentColor" stroke-width="6" stroke-linecap="round" fill="none">
            <path d="M16 14 V36" /><path d="M32 14 V36" /><path d="M48 14 V36" />
            <path d="M16 36 H48" /><path d="M32 36 V70" />
          </g>
        </svg>
        <h1 class="welcome-title">Welcome to Tine</h1>
        <p class="welcome-lede">
          A fast, local outliner for your <strong>Logseq</strong> graph. Tine reads and writes the
          same Markdown files — so you can keep using Logseq too, on the same notes.
        </p>

        <div class="welcome-actions">
          <button
            class="welcome-choice"
            disabled={!!busy()}
            onClick={run("open", switchGraph)}
          >
            <svg class="welcome-choice-ic" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M3 7a2 2 0 0 1 2-2h3.6l2 2H19a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"
                fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round" />
            </svg>
            <span class="welcome-choice-text">
              <span class="welcome-choice-title">Open an existing graph</span>
              <span class="welcome-choice-sub">Point Tine at a Logseq graph folder you already have.</span>
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
            <span class="welcome-choice-text">
              <span class="welcome-choice-title">Create a new graph</span>
              <span class="welcome-choice-sub">
                New to this? We'll set up a small demo graph that shows you around.
              </span>
            </span>
          </button>
        </div>

        <Show when={busy() === "create"}>
          <p class="welcome-busy">Setting up your graph…</p>
        </Show>
        <Show when={busy() === "open"}>
          <p class="welcome-busy">Opening…</p>
        </Show>

        <p class="welcome-foot">
          Free &amp; open source · your notes stay plain Markdown files you own.
        </p>
      </div>
    </div>
  );
}

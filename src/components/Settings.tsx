import { For, Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import {
  settingsOpen,
  closeSettings,
  theme,
  toggleTheme,
  workflow,
  graphMeta,
  shortcutOverrides,
  setShortcutOverride,
  resetShortcutOverride,
  accentColor,
  changeAccent,
  wideMode,
  toggleWideMode,
  documentMode,
  toggleDocumentMode,
  dimInFocus,
  setDimInFocus,
} from "../ui";
import { commandDefaults, eventToBindingString, setKeybindingsSuspended } from "../keybindings";
import { switchGraph } from "../graph";
import { backend } from "../backend";

export function Settings(): JSX.Element {
  const [publishMsg, setPublishMsg] = createSignal("");
  const doPublish = async () => {
    setPublishMsg("Exporting…");
    try {
      const [dir, n] = await backend().publishHtml();
      setPublishMsg(`Exported ${n} pages to ${dir}`);
    } catch (e) {
      setPublishMsg(`Failed: ${String(e)}`);
    }
  };

  // Effective binding = local override > config.edn > built-in default.
  const shortcuts = () => {
    const cfg = graphMeta()?.shortcuts ?? {};
    const ov = shortcutOverrides();
    return commandDefaults().map((c) => ({
      ...c,
      effective: ov[c.id] ?? cfg[c.id] ?? c.binding,
      overridden: c.id in ov,
    }));
  };

  // Recording: capture the next chord for the command being remapped.
  const [recording, setRecording] = createSignal<string | null>(null);
  createEffect(() => {
    const id = recording();
    if (!id) {
      setKeybindingsSuspended(false);
      return;
    }
    setKeybindingsSuspended(true);
    onCleanup(() => setKeybindingsSuspended(false));
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecording(null);
        return;
      }
      const b = eventToBindingString(e);
      if (!b) return; // bare modifier — keep waiting
      setShortcutOverride(id, b);
      setRecording(null);
    };
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => window.removeEventListener("keydown", onKey, true));
  });

  return (
    <Show when={settingsOpen()}>
      <div class="modal-overlay" onClick={closeSettings}>
        <div class="settings-modal" onClick={(e) => e.stopPropagation()}>
          <div class="settings-head">
            <span>Settings</span>
            <button class="icon-btn" onClick={closeSettings}>
              ✕
            </button>
          </div>

          <div class="settings-row">
            <span class="settings-label">Theme</span>
            <div class="settings-seg">
              <button classList={{ active: theme() === "light" }} onClick={() => theme() !== "light" && toggleTheme()}>
                Light
              </button>
              <button classList={{ active: theme() === "dark" }} onClick={() => theme() !== "dark" && toggleTheme()}>
                Dark
              </button>
            </div>
          </div>

          <div class="settings-row">
            <span class="settings-label">Accent color</span>
            <div>
              <input
                type="color"
                class="settings-color"
                value={accentColor() ?? "#2563eb"}
                onInput={(e) => changeAccent(e.currentTarget.value)}
              />
              <Show when={accentColor()}>
                <button class="settings-btn" style={{ "margin-left": "8px" }} onClick={() => changeAccent(null)}>
                  Reset
                </button>
              </Show>
            </div>
          </div>

          <div class="settings-row">
            <span class="settings-label">Wide mode</span>
            <div>
              <button
                class="settings-toggle"
                classList={{ on: wideMode() }}
                role="switch"
                aria-checked={wideMode()}
                onClick={toggleWideMode}
              >
                <span class="settings-toggle-knob" />
              </button>
              <span class="settings-hint" style={{ "margin-left": "8px" }}>
                Drops the reading-width cap.
              </span>
            </div>
          </div>

          <div class="settings-row">
            <span class="settings-label">Document mode</span>
            <div>
              <button
                class="settings-toggle"
                classList={{ on: documentMode() }}
                role="switch"
                aria-checked={documentMode()}
                onClick={toggleDocumentMode}
              >
                <span class="settings-toggle-knob" />
              </button>
              <span class="settings-hint" style={{ "margin-left": "8px" }}>
                Hides bullets and indent guides for a cleaner prose view.
              </span>
            </div>
          </div>

          <div class="settings-row">
            <span class="settings-label">Dim in focus mode</span>
            <div>
              <button
                class="settings-toggle"
                classList={{ on: dimInFocus() }}
                role="switch"
                aria-checked={dimInFocus()}
                onClick={() => setDimInFocus(!dimInFocus())}
              >
                <span class="settings-toggle-knob" />
              </button>
              <span class="settings-hint" style={{ "margin-left": "8px" }}>
                Auto-enable dim inactive blocks (t b) when entering focus mode (t f).
              </span>
            </div>
          </div>

          <div class="settings-row">
            <span class="settings-label">Task workflow</span>
            <span class="settings-value">
              {workflow() === "now" ? "NOW / LATER" : "TODO / DOING"}{" "}
              <span class="settings-hint">(set :preferred-workflow in config.edn)</span>
            </span>
          </div>

          <div class="settings-row">
            <span class="settings-label">Graph</span>
            <div>
              <span class="settings-value mono">{graphMeta()?.root ?? "—"}</span>
              <div style={{ "margin-top": "6px" }}>
                <button class="settings-btn" onClick={() => void switchGraph()}>
                  Open another graph…
                </button>
              </div>
            </div>
          </div>

          <div class="settings-row">
            <span class="settings-label">Publish</span>
            <div>
              <button class="settings-btn" onClick={doPublish}>
                Export graph to HTML
              </button>
              <Show when={publishMsg()}>
                <div class="settings-hint" style={{ "margin-top": "4px" }}>
                  {publishMsg()}
                </div>
              </Show>
            </div>
          </div>

          <div class="settings-section">Keyboard shortcuts</div>
          <div class="settings-hint settings-block">
            Click a binding to record new keys ("mod" = Ctrl). Esc cancels.
            Overrides are saved locally on top of <code>config.edn</code>.
          </div>
          <div class="settings-shortcuts">
            <For each={shortcuts()}>
              {(s) => (
                <div class="settings-shortcut-row">
                  <span class="settings-shortcut-label">{s.label}</span>
                  <button
                    class="settings-shortcut-binding"
                    classList={{ recording: recording() === s.id, overridden: s.overridden }}
                    onClick={() => setRecording(recording() === s.id ? null : s.id)}
                    title="Click to remap"
                  >
                    {recording() === s.id ? "Press keys…" : s.effective}
                  </button>
                  <span class="settings-shortcut-tail">
                    <Show when={s.overridden}>
                      <button
                        class="settings-shortcut-reset"
                        title="Reset to default"
                        onClick={() => resetShortcutOverride(s.id)}
                      >
                        ↺
                      </button>
                    </Show>
                    <span class="settings-shortcut-id mono">{s.id}</span>
                  </span>
                </div>
              )}
            </For>
          </div>
        </div>
      </div>
    </Show>
  );
}

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
} from "../ui";
import { commandDefaults, eventToBindingString, setKeybindingsSuspended } from "../keybindings";
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
            <span class="settings-label">Task workflow</span>
            <span class="settings-value">
              {workflow() === "now" ? "NOW / LATER" : "TODO / DOING"}{" "}
              <span class="settings-hint">(set :preferred-workflow in config.edn)</span>
            </span>
          </div>

          <div class="settings-row">
            <span class="settings-label">Graph</span>
            <span class="settings-value mono">{graphMeta()?.root ?? "—"}</span>
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

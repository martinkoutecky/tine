import { Show, type JSX } from "solid-js";
import { settingsOpen, closeSettings, theme, toggleTheme, workflow, graphMeta } from "../ui";

export function Settings(): JSX.Element {
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

          <div class="settings-section">Keyboard shortcuts</div>
          <div class="settings-hint settings-block">
            Defaults: <code>mod+k</code> search · <code>g j</code> journals · <code>t t</code> theme ·{" "}
            <code>t l</code> sidebar · <code>mod+z</code>/<code>mod+shift+z</code> undo/redo ·{" "}
            <code>mod+enter</code> cycle task · <code>Tab</code>/<code>Shift+Tab</code> indent ·{" "}
            <code>Esc</code> select block. Override in config.edn under{" "}
            <code>:shortcuts</code>.
          </div>
        </div>
      </div>
    </Show>
  );
}

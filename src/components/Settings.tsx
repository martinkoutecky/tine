import { For, Show, createSignal, type JSX } from "solid-js";
import { settingsOpen, closeSettings, theme, toggleTheme, workflow, graphMeta } from "../ui";
import { currentShortcuts } from "../keybindings";
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
          <div class="settings-shortcuts">
            <For each={currentShortcuts()}>
              {(s) => (
                <div class="settings-shortcut-row">
                  <span class="settings-shortcut-label">{s.label}</span>
                  <code class="settings-shortcut-binding">{s.binding}</code>
                  <span class="settings-shortcut-id mono">{s.id}</span>
                </div>
              )}
            </For>
          </div>
          <div class="settings-hint settings-block">
            Remap any of these in <code>config.edn</code> under <code>:shortcuts</code>,
            e.g. <code>{`:shortcuts {:editor/move-block-down "alt+shift+down"}`}</code>.
            "mod" = Ctrl. Set a binding to <code>"false"</code> to disable it.
          </div>
        </div>
      </div>
    </Show>
  );
}

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
  carryKeepsContext,
  setCarryKeepsContext,
  carryHeader,
  setCarryHeader,
  carryDays,
  setCarryDays,
  showCarryButtons,
  setShowCarryButtons,
  pushToast,
} from "../ui";
import { commandDefaults, eventToBindingString, setKeybindingsSuspended } from "../keybindings";
import { switchGraph, loadGraphPath } from "../graph";
import { flushAll } from "../store";
import { backend, type BackupInfo } from "../backend";

type Tab = "appearance" | "tasks" | "backups" | "graph" | "shortcuts";
const TABS: { id: Tab; label: string }[] = [
  { id: "appearance", label: "Appearance" },
  { id: "tasks", label: "Tasks & carry-over" },
  { id: "backups", label: "Backups" },
  { id: "graph", label: "Graph" },
  { id: "shortcuts", label: "Keyboard shortcuts" },
];

export function Settings(): JSX.Element {
  const [tab, setTab] = createSignal<Tab>("appearance");
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
          <aside class="settings-nav">
            <div class="settings-nav-title">Settings</div>
            <For each={TABS}>
              {(t) => (
                <button
                  class="settings-nav-item"
                  classList={{ active: tab() === t.id }}
                  onClick={() => setTab(t.id)}
                >
                  {t.label}
                </button>
              )}
            </For>
            <div class="settings-nav-foot">Built {buildStamp()}</div>
          </aside>

          <div class="settings-pane">
            <div class="settings-pane-head">
              <span>{TABS.find((t) => t.id === tab())?.label}</span>
              <button class="icon-btn" onClick={closeSettings}>
                ✕
              </button>
            </div>
            <div class="settings-pane-body">
              <Show when={tab() === "appearance"}>
                <AppearanceTab />
              </Show>
              <Show when={tab() === "tasks"}>
                <TasksTab />
              </Show>
              <Show when={tab() === "backups"}>
                <BackupsTab />
              </Show>
              <Show when={tab() === "graph"}>
                <GraphTab publishMsg={publishMsg()} doPublish={doPublish} />
              </Show>
              <Show when={tab() === "shortcuts"}>
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
              </Show>
            </div>
          </div>
        </div>
      </div>
    </Show>
  );
}

function Toggle(props: { on: boolean; onClick: () => void }): JSX.Element {
  return (
    <button
      class="settings-toggle"
      classList={{ on: props.on }}
      role="switch"
      aria-checked={props.on}
      onClick={props.onClick}
    >
      <span class="settings-toggle-knob" />
    </button>
  );
}

function AppearanceTab(): JSX.Element {
  return (
    <>
      <div class="settings-row">
        <span class="settings-label">Theme</span>
        <button
          class="theme-switch"
          classList={{ "is-dark": theme() === "dark" }}
          role="switch"
          aria-checked={theme() === "dark"}
          title="Toggle light / dark (t t)"
          onClick={toggleTheme}
        >
          <span class="theme-opt">☀ Light</span>
          <span class="theme-opt">☾ Dark</span>
          <span class="theme-knob" />
        </button>
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
          <Toggle on={wideMode()} onClick={toggleWideMode} />
          <span class="settings-hint" style={{ "margin-left": "8px" }}>
            Drops the reading-width cap.
          </span>
        </div>
      </div>

      <div class="settings-row">
        <span class="settings-label">Document mode</span>
        <div>
          <Toggle on={documentMode()} onClick={toggleDocumentMode} />
          <span class="settings-hint" style={{ "margin-left": "8px" }}>
            Hides bullets and indent guides for a cleaner prose view.
          </span>
        </div>
      </div>

      <div class="settings-row">
        <span class="settings-label">Dim in focus mode</span>
        <div>
          <Toggle on={dimInFocus()} onClick={() => setDimInFocus(!dimInFocus())} />
          <span class="settings-hint" style={{ "margin-left": "8px" }}>
            Auto-enable dim inactive blocks (t b) when entering focus mode (t f).
          </span>
        </div>
      </div>
    </>
  );
}

function TasksTab(): JSX.Element {
  return (
    <>
      <div class="settings-row">
        <span class="settings-label">Show carry-over buttons</span>
        <div>
          <Toggle on={showCarryButtons()} onClick={() => setShowCarryButtons(!showCarryButtons())} />
          <span class="settings-hint" style={{ "margin-left": "8px" }}>
            Show the carry buttons next to journal titles. Off → use the right-click menu instead.
          </span>
        </div>
      </div>

      <div class="settings-row">
        <span class="settings-label">Carry-over keeps context</span>
        <div>
          <Toggle on={carryKeepsContext()} onClick={() => setCarryKeepsContext(!carryKeepsContext())} />
          <span class="settings-hint" style={{ "margin-left": "8px" }}>
            Move whole blocks that contain an open task (on) vs. pull out just the task (off).
          </span>
        </div>
      </div>

      <div class="settings-row">
        <span class="settings-label">Carry-over header</span>
        <div>
          <Toggle on={carryHeader()} onClick={() => setCarryHeader(!carryHeader())} />
          <span class="settings-hint" style={{ "margin-left": "8px" }}>
            Add a “Carried over” heading above carried tasks.
          </span>
        </div>
      </div>

      <div class="settings-row">
        <span class="settings-label">Carry “last N days”</span>
        <div>
          <input
            type="number"
            min="1"
            max="3650"
            class="settings-num"
            value={carryDays()}
            onChange={(e) => setCarryDays(Number(e.currentTarget.value))}
          />
          <span class="settings-hint" style={{ "margin-left": "8px" }}>
            N for the “Carry last N days” button on today’s journal (and the Ctrl-K command).
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
    </>
  );
}

function GraphTab(props: { publishMsg: string; doPublish: () => void }): JSX.Element {
  return (
    <>
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
          <button class="settings-btn" onClick={props.doPublish}>
            Export graph to HTML
          </button>
          <Show when={props.publishMsg}>
            <div class="settings-hint" style={{ "margin-top": "4px" }}>
              {props.publishMsg}
            </div>
          </Show>
        </div>
      </div>
    </>
  );
}

function BackupsTab(): JSX.Element {
  const [keep, setKeep] = createSignal(12);
  const [list, setList] = createSignal<BackupInfo[]>([]);
  const [busy, setBusy] = createSignal(false);

  const refresh = async () => {
    try {
      setList(await backend().listBackups());
    } catch {
      setList([]);
    }
  };

  // Load the current keep count + snapshot list when this tab mounts.
  createEffect(() => {
    void backend().getBackupKeep().then(setKeep).catch(() => {});
    void refresh();
  });

  const saveKeep = async (n: number) => {
    const v = Math.max(1, Math.min(1000, Math.floor(n) || 12));
    setKeep(v);
    try {
      await backend().setBackupKeep(v);
      void refresh(); // a lower cap prunes immediately on the Rust side
    } catch (e) {
      pushToast(`Couldn't save: ${String(e)}`, "error");
    }
  };

  const restore = async (b: BackupInfo) => {
    const when = fmtStamp(b.stamp);
    if (
      !confirm(
        `Restore the snapshot from ${when}?\n\n` +
          `This overwrites journals/ and pages/ with the ${b.files} file(s) in that backup. ` +
          `Your current state is snapshotted first, so this is reversible.`
      )
    )
      return;
    setBusy(true);
    try {
      // Persist current edits first so the pre-restore safety snapshot captures
      // them (and the reload below doesn't write stale edits over the restore).
      // Abort if a page couldn't be saved rather than discard it.
      if (!(await flushAll())) {
        pushToast("Some pages couldn't be saved — resolve conflicts before restoring.", "error");
        setBusy(false);
        return;
      }
      await backend().restoreBackup(b.stamp);
      const root = graphMeta()?.root ?? "";
      await loadGraphPath(root); // reopen from the restored files
      pushToast(`Restored snapshot from ${when}`, "success");
      void refresh();
    } catch (e) {
      pushToast(`Restore failed: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <div class="settings-hint settings-block">
        Tine snapshots your graph’s markdown to a local folder each time it opens
        (outside the graph, so Syncthing never sees it). A safety net against a bad
        write — independent of OG Logseq’s own backups.
      </div>

      <div class="settings-row">
        <span class="settings-label">Snapshots to keep</span>
        <div>
          <input
            type="number"
            min="1"
            max="1000"
            class="settings-num"
            value={keep()}
            onChange={(e) => void saveKeep(Number(e.currentTarget.value))}
          />
          <span class="settings-hint" style={{ "margin-left": "8px" }}>
            Oldest snapshots beyond this are pruned.
          </span>
        </div>
      </div>

      <div class="settings-section">
        Available snapshots
        <button class="settings-btn" style={{ "margin-left": "10px" }} onClick={() => void refresh()}>
          Refresh
        </button>
      </div>
      <Show
        when={list().length}
        fallback={<div class="settings-hint settings-block">No snapshots yet.</div>}
      >
        <div class="settings-backups">
          <For each={list()}>
            {(b) => (
              <div class="settings-backup-row">
                <span class="settings-backup-when">{fmtStamp(b.stamp)}</span>
                <span class="settings-backup-files mono">{b.files} files</span>
                <button class="settings-btn" disabled={busy()} onClick={() => void restore(b)}>
                  Restore
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>
    </>
  );
}

// Build timestamp (stamped by Vite at bundle time) — handy for confirming the
// running binary is the latest, not a stale Syncthing copy.
function buildStamp(): string {
  try {
    return new Date(__BUILD_TIME__).toLocaleString();
  } catch {
    return __BUILD_TIME__;
  }
}

// "2026-06-17_14-30-05" (UTC) → a readable local timestamp.
function fmtStamp(s: string): string {
  const m = s.match(/^(\d{4})-(\d{2})-(\d{2})_(\d{2})-(\d{2})-(\d{2})$/);
  if (!m) return s;
  const d = new Date(`${m[1]}-${m[2]}-${m[3]}T${m[4]}:${m[5]}:${m[6]}Z`);
  return isNaN(d.getTime()) ? s : d.toLocaleString();
}

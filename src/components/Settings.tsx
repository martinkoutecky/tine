import { For, Show, createEffect, createResource, createSignal, onCleanup, type JSX } from "solid-js";
import {
  settingsOpen,
  closeSettings,
  setJournalTemplate,
  theme,
  toggleTheme,
  workflow,
  changeWorkflow,
  changePreferredFormat,
  changeJournalTitleFormat,
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
  changeStartOfWeek,
  carryKeepsContext,
  setCarryKeepsContext,
  carryHeader,
  setCarryHeader,
  carryDays,
  setCarryDays,
  showCarryButtons,
  setShowCarryButtons,
  agendaDaysBack,
  setAgendaDaysBack,
  agendaDaysAhead,
  setAgendaDaysAhead,
  pushToast,
  journalConflicts,
  refreshJournalConflicts,
} from "../ui";
import { interfaceZoom, zoomIn, zoomOut, zoomReset } from "../zoom";
import { smoothScrollEnabled, setSmoothScroll } from "../smoothScroll";
import {
  copyIncludeSubtree,
  setCopyIncludeSubtree,
  copyStripCollapsed,
  setCopyStripCollapsed,
} from "../copySettings";
import { linkFirstMatch, setLinkFirstMatch } from "../editor/linkDefault";
import { openPage, openFile } from "../router";
import { commandDefaults, eventToBindingString, setKeybindingsSuspended } from "../keybindings";
import { switchGraph, loadGraphPath } from "../graph";
import { flushAll } from "../store";
import { backend, type BackupInfo } from "../backend";
import type { AssetInfo, TrashStats, JournalFile } from "../types";
import { formatJournal } from "../journal";

// Journal display-title formats offered in the date-format dropdown — OG's
// `journal-title-formatters` set (frontend/date.cljs). Display-only; the on-disk
// journal file name is governed separately by `:journal/file-name-format`.
const DATE_FORMATS = [
  "MMM do, yyyy",
  "MMMM do, yyyy",
  "do MMM yyyy",
  "do MMMM yyyy",
  "E, dd-MM-yyyy",
  "EEE, dd-MM-yyyy",
  "EEEE, dd-MM-yyyy",
  "EEE, MM/dd/yyyy",
  "EEEE, MM/dd/yyyy",
  "EEE, yyyy/MM/dd",
  "dd-MM-yyyy",
  "MM/dd/yyyy",
  "MM-dd-yyyy",
  "MM_dd_yyyy",
  "yyyy/MM/dd",
  "yyyy-MM-dd",
  "yyyy-MM-dd EEEE",
  "yyyy_MM_dd",
  "yyyyMMdd",
];

type Tab = "appearance" | "tasks" | "backups" | "graph" | "shortcuts";
const TABS: { id: Tab; label: string }[] = [
  { id: "appearance", label: "Appearance" },
  { id: "tasks", label: "Journals & tasks" },
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
                <AssetsTab />
                <JournalConflictsPanel />
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

// One setting: label + control on a line, with the explanatory hint on its own
// full-width line below (so long hints read cleanly instead of being squeezed
// into the right column). Pass `hint` as JSX to allow inline <code>/markup.
function Field(props: { label: string; hint?: JSX.Element; children: JSX.Element }): JSX.Element {
  return (
    <div class="settings-field">
      <div class="settings-field-row">
        <span class="settings-label">{props.label}</span>
        <div class="settings-field-control">{props.children}</div>
      </div>
      <Show when={props.hint}>
        <div class="settings-hint settings-field-hint">{props.hint}</div>
      </Show>
    </div>
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

// A settings row for an option where Tine's behavior can DIFFER from Logseq. Shows
// a "Differs from Logseq" chip + a one-line note on what Logseq does whenever the
// current value isn't the OG one, plus a "Match Logseq" button to flip back. Use
// this (instead of a plain Field) so non-OG defaults are always visible and one
// click from reverting. `ogValue` is the toggle state that matches Logseq.
function OgField(props: {
  label: string;
  hint?: JSX.Element;
  ogNote: string;
  ogValue: boolean;
  on: boolean;
  onToggle: () => void;
}): JSX.Element {
  const diverges = () => props.on !== props.ogValue;
  return (
    <div class="settings-field og-field" classList={{ "og-diverges": diverges() }}>
      <div class="settings-field-row">
        <span class="settings-label">
          {props.label}
          <Show when={diverges()}>
            <span class="og-badge" title="Tine's default differs from Logseq here">Differs from Logseq</span>
          </Show>
        </span>
        <div class="settings-field-control">
          <Toggle on={props.on} onClick={props.onToggle} />
        </div>
      </div>
      <Show when={props.hint}>
        <div class="settings-hint settings-field-hint">{props.hint}</div>
      </Show>
      <div class="og-note">
        <span class="og-logseq">Logseq: {props.ogNote}</span>
        <Show when={diverges()}>
          <button class="og-revert" onClick={props.onToggle}>↩ Match Logseq</button>
        </Show>
      </div>
    </div>
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
          <span class="theme-opt"><span class="theme-ico">☀</span>Light</span>
          <span class="theme-opt"><span class="theme-ico">☾</span>Dark</span>
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

      <Field
        label="Interface size"
        hint="Zoom the whole interface — Ctrl + / Ctrl − / Ctrl 0, or Ctrl + scroll. Saved on this device. (Over/within the PDF pane, those zoom the PDF instead.)"
      >
        <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
          <button class="settings-btn" title="Smaller (Ctrl −)" onClick={zoomOut}>−</button>
          <span class="mono" style={{ "min-width": "3.4em", "text-align": "center" }}>
            {Math.round(interfaceZoom() * 100)}%
          </span>
          <button class="settings-btn" title="Larger (Ctrl +)" onClick={zoomIn}>+</button>
          <Show when={interfaceZoom() !== 1}>
            <button class="settings-btn" style={{ "margin-left": "8px" }} onClick={zoomReset}>
              Reset
            </button>
          </Show>
        </div>
      </Field>

      <Field label="Wide mode" hint="Drops the reading-width cap.">
        <Toggle on={wideMode()} onClick={toggleWideMode} />
      </Field>

      <Field label="Document mode" hint="Hides bullets and indent guides for a cleaner prose view.">
        <Toggle on={documentMode()} onClick={toggleDocumentMode} />
      </Field>

      <Field
        label="Dim in focus mode"
        hint="Auto-enable dim inactive blocks (t b) when entering focus mode (t f)."
      >
        <Toggle on={dimInFocus()} onClick={() => setDimInFocus(!dimInFocus())} />
      </Field>

      <Field
        label="Smooth scrolling (experimental)"
        hint="Animate the journal feed's scrolling to smooth out WebKitGTK's stepped mouse-wheel jumps. Off by default; this is a feel experiment — turn it off if it gets in the way."
      >
        <Toggle on={smoothScrollEnabled()} onClick={() => setSmoothScroll(!smoothScrollEnabled())} />
      </Field>

      <Field
        label="First day of week"
        hint={
          <>
            Starting column of the calendar and the scheduled/deadline date
            pickers. Saved to <code>:start-of-week</code> in <code>config.edn</code>
            (Logseq’s setting), so it travels with the graph.
          </>
        }
      >
        <select
          class="settings-select"
          value={String(graphMeta()?.start_of_week ?? 6)}
          onChange={(e) => changeStartOfWeek(Number(e.currentTarget.value))}
        >
          {/* Logseq convention: 0=Monday … 6=Sunday. */}
          <option value="0">Monday</option>
          <option value="1">Tuesday</option>
          <option value="2">Wednesday</option>
          <option value="3">Thursday</option>
          <option value="4">Friday</option>
          <option value="5">Saturday</option>
          <option value="6">Sunday</option>
        </select>
      </Field>
    </>
  );
}

// New-journal template picker: a dropdown of all `template::` templates + "(none)
// = blank days" (the factory default — clearing the pointer), and an "Edit →" jump
// to the chosen template's block. Uses existing concepts only: templates + the
// config pointer. No catalogue, no built-in default.
function JournalTemplateField(): JSX.Element {
  const [templates] = createResource(() => backend().listTemplates());
  const current = () => graphMeta()?.default_journal_template ?? "";
  const list = () => templates() ?? [];
  const selected = () => list().find((t) => t.name === current());
  // A configured name that no longer matches a template (stale pointer) — surface
  // it so the dropdown reflects config rather than silently showing "(none)".
  const missing = () => current() !== "" && !list().some((t) => t.name === current());
  return (
    <Field
      label="New-journal template"
      hint={
        <>
          Template inserted into a new day's journal. Saved to{" "}
          <code>:default-templates {"{:journals …}"}</code> in <code>config.edn</code>. “(none)” →
          blank days (the default). Make a template via a block's right-click menu.
        </>
      }
    >
      <div class="settings-jtmpl">
        <select
          class="settings-select"
          value={current()}
          onChange={(e) => setJournalTemplate(e.currentTarget.value || null)}
        >
          <option value="">(none) — blank days</option>
          <Show when={missing()}>
            <option value={current()}>{current()} (not found)</option>
          </Show>
          <For each={list()}>{(t) => <option value={t.name}>{t.name}</option>}</For>
        </select>
        <Show when={selected()}>
          {(t) => (
            <button
              class="settings-link"
              onClick={() => {
                openPage(t().page, t().kind);
                closeSettings();
              }}
            >
              Edit →
            </button>
          )}
        </Show>
      </div>
    </Field>
  );
}

/** Journal display-title format picker. Shows each pattern with a live example
 *  (today rendered in it). Includes the graph's current value even if it isn't
 *  one of the presets, so a hand-edited config.edn round-trips. */
function DateFormatSelect(): JSX.Element {
  const today = new Date();
  const current = () => graphMeta()?.journal_page_title_format || "MMM do, yyyy";
  const options = () => (DATE_FORMATS.includes(current()) ? DATE_FORMATS : [current(), ...DATE_FORMATS]);
  return (
    <select
      class="settings-select"
      value={current()}
      onChange={(e) => changeJournalTitleFormat(e.currentTarget.value)}
    >
      <For each={options()}>
        {(fmt) => <option value={fmt}>{`${fmt}  —  ${formatJournal(today, fmt)}`}</option>}
      </For>
    </select>
  );
}

function TasksTab(): JSX.Element {
  // Quick-capture Enter behaviour (app-level setting, read by the capture window).
  const [captureEnterFiles, setCaptureEnterFiles] = createSignal(false);
  void backend()
    .getCaptureEnterFiles()
    .then(setCaptureEnterFiles)
    .catch(() => {});
  const toggleCaptureEnter = () => {
    const v = !captureEnterFiles();
    setCaptureEnterFiles(v);
    void backend().setCaptureEnterFiles(v).catch(() => {});
  };
  return (
    <>
      <Field
        label="File format"
        hint={
          <>
            Format for <strong>new</strong> pages and journals — Markdown or Org. Existing{" "}
            <code>.md</code> and <code>.org</code> files keep their own format and are edited in
            place. Saved to <code>:preferred-format</code> in <code>config.edn</code>.
          </>
        }
      >
        <div class="settings-segment">
          <button
            classList={{ active: (graphMeta()?.preferred_format ?? "md") === "md" }}
            onClick={() => changePreferredFormat("md")}
          >
            Markdown
          </button>
          <button
            classList={{ active: graphMeta()?.preferred_format === "org" }}
            onClick={() => changePreferredFormat("org")}
          >
            Org
          </button>
        </div>
      </Field>

      <Field
        label="Journal date format"
        hint={
          <>
            How journal dates are displayed and how new <code>[[date]]</code> titles are written.
            Display-only — your journal <em>file names</em> are untouched and existing journals keep
            working. Saved to <code>:journal/page-title-format</code>.
          </>
        }
      >
        <DateFormatSelect />
      </Field>

      <Field
        label="Show carry-over buttons"
        hint="Show the carry buttons next to journal titles. Off → use the right-click menu instead."
      >
        <Toggle on={showCarryButtons()} onClick={() => setShowCarryButtons(!showCarryButtons())} />
      </Field>

      <Field
        label="Carry-over keeps context"
        hint="Move whole blocks that contain an open task (on) vs. pull out just the task (off)."
      >
        <Toggle on={carryKeepsContext()} onClick={() => setCarryKeepsContext(!carryKeepsContext())} />
      </Field>

      <Field label="Carry-over header" hint="Add a “Carried over” heading above carried tasks.">
        <Toggle on={carryHeader()} onClick={() => setCarryHeader(!carryHeader())} />
      </Field>

      <Field
        label="Carry “last N days”"
        hint="N for the “Carry last N days” button on today’s journal (and the Ctrl-K command)."
      >
        <input
          type="number"
          min="1"
          max="3650"
          class="settings-num"
          value={carryDays()}
          onChange={(e) => setCarryDays(Number(e.currentTarget.value))}
        />
      </Field>

      <Field
        label="Task workflow"
        hint={
          <>
            Which markers Tab/⌘↵ cycle through and what new tasks use. Saved to{" "}
            <code>:preferred-workflow</code> in <code>config.edn</code>, so it travels with the
            graph.
          </>
        }
      >
        <div class="settings-segment">
          <button
            classList={{ active: workflow() === "todo" }}
            onClick={() => changeWorkflow("todo")}
          >
            TODO / DOING
          </button>
          <button classList={{ active: workflow() === "now" }} onClick={() => changeWorkflow("now")}>
            NOW / LATER
          </button>
        </div>
      </Field>

      <JournalTemplateField />

      <Field
        label="Quick-capture Enter key"
        hint={`In the quick-capture window: ON → Enter files the capture. OFF → Enter starts a new block; the “Quick-capture: file to today’s journal” shortcut files (default Ctrl+Shift+Enter, remappable under Keyboard shortcuts). Ctrl+Enter stays free for cycling the task marker.`}
      >
        <Toggle on={captureEnterFiles()} onClick={toggleCaptureEnter} />
      </Field>

      <Field
        label="Link autocomplete default"
        hint={`When you type [[name (or #name) that isn’t an exact existing page: ON → Enter LINKS to the first match (and “Create…” moves to the end of the list); OFF (default, like Logseq) → Enter CREATES a new page/tag unless an exact match exists. Either way the arrow keys reach the other options.`}
      >
        <Toggle on={linkFirstMatch()} onClick={() => setLinkFirstMatch(!linkFirstMatch())} />
      </Field>

      <OgField
        label="Copy a parent block's sub-blocks"
        hint="When you copy/cut a selected block that has children: ON copies the whole sub-tree; OFF copies only the block(s) you actually selected. Tine defaults to OFF because selecting just the parent and getting its entire tree is surprising."
        ogNote="always copies a selected block's whole sub-tree."
        ogValue={true}
        on={copyIncludeSubtree()}
        onToggle={() => setCopyIncludeSubtree(!copyIncludeSubtree())}
      />

      <OgField
        label="Strip collapsed:: when copying"
        hint="A collapsed block carries a hidden collapsed:: true property (view state, not content). Tine defaults to ON (drops it from copied text for a cleaner paste); OFF keeps it. (id:: is always stripped from copies, like Logseq.)"
        ogNote="keeps collapsed:: in the copied text (only id:: is stripped)."
        ogValue={false}
        on={copyStripCollapsed()}
        onToggle={() => setCopyStripCollapsed(!copyStripCollapsed())}
      />

      <Field
        label="Agenda window"
        hint={
          <>
            Today’s “Scheduled &amp; Deadline” list shows items whose scheduled/deadline date is
            within this window; older/further ones are hidden.
          </>
        }
      >
        <input
          type="number"
          min="0"
          max="3650"
          class="settings-num"
          value={agendaDaysBack()}
          onChange={(e) => setAgendaDaysBack(Number(e.currentTarget.value))}
        />
        <span class="settings-hint">days back ·</span>
        <input
          type="number"
          min="0"
          max="3650"
          class="settings-num"
          value={agendaDaysAhead()}
          onChange={(e) => setAgendaDaysAhead(Number(e.currentTarget.value))}
        />
        <span class="settings-hint">days ahead</span>
      </Field>
    </>
  );
}

function GraphTab(props: { publishMsg: string; doPublish: () => void }): JSX.Element {
  // File-watch mechanism (device-local). Loaded from the backend on mount.
  const [watchMode, setWatchMode] = createSignal<"inotify" | "poll">("inotify");
  void backend()
    .getWatchMode()
    .then((m) => setWatchMode(m === "poll" ? "poll" : "inotify"))
    .catch(() => {});
  const changeWatchMode = (m: "inotify" | "poll") => {
    if (m === watchMode()) return;
    setWatchMode(m);
    void backend().setWatchMode(m).catch(() => {});
  };
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

      <Field
        label="Watch for external edits"
        hint={
          <>
            How Tine notices changes made outside it (OG Logseq, Syncthing, an
            editor). <b>Live</b> uses the OS file watcher (inotify) — no idle CPU
            wakeups; the right choice on a normal local disk. <b>Poll</b> rescans
            every 3 seconds — only needed on filesystems where inotify is
            unreliable (some network/NFS mounts). Saved per device.
          </>
        }
      >
        <div class="settings-segment">
          <button
            classList={{ active: watchMode() === "inotify" }}
            onClick={() => changeWatchMode("inotify")}
          >
            Live (inotify)
          </button>
          <button
            classList={{ active: watchMode() === "poll" }}
            onClick={() => changeWatchMode("poll")}
          >
            Poll (3s)
          </button>
        </div>
      </Field>

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
    // Native GTK confirm — window.confirm silently returns true here, which would
    // overwrite the graph with no prompt.
    if (
      !(await backend().confirm(
        `Restore the snapshot from ${when}?\n\n` +
          `This overwrites journals/ and pages/ with the ${b.files} file(s) in that backup. ` +
          `Your current state is snapshotted first, so this is reversible.`
      ))
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

      <Field label="Snapshots to keep" hint="Oldest snapshots beyond this are pruned.">
        <input
          type="number"
          min="1"
          max="1000"
          class="settings-num"
          value={keep()}
          onChange={(e) => void saveKeep(Number(e.currentTarget.value))}
        />
      </Field>

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

// Orphaned-media cleanup: scan (on demand — it parses the whole graph) for
// assets/ files no block links to, and let the user move them to the recoverable
// trash. Tine never auto-deletes media (a deleted block keeps its files), so this
// is how unused media gets cleaned up.
// One file in a duplicate-day conflict. Click the name to reveal its full
// contents; the action buttons let you reach and reconcile it (#21): Open
// navigates to THIS specific file (editable, saves back to itself), Merge folds a
// stray into the canonical day, Rename rescues it as a normal page, Trash removes
// the redundant one (recoverable).
function ConflictFileRow(props: {
  file: JournalFile;
  onOpen: () => void;
  onMerge?: () => void;
  onRename: (newName: string) => void;
  onTrash: () => void;
}): JSX.Element {
  const [open, setOpen] = createSignal(false);
  const [renaming, setRenaming] = createSignal(false);
  const [newName, setNewName] = createSignal("");
  const [content] = createResource(
    () => (open() ? props.file.name : null),
    async (name) => (name ? backend().readJournalFile(name).catch((e) => `(couldn’t read: ${String(e)})`) : "")
  );
  const submitRename = () => {
    const n = newName().trim();
    if (n) props.onRename(n);
    setRenaming(false);
    setNewName("");
  };
  return (
    <>
      <div class="journal-conflict-row">
        <button class="settings-asset-name mono" title="Show this file's contents" onClick={() => setOpen(!open())}>
          {open() ? "▾ " : "▸ "}
          {props.file.name}
          <Show when={props.file.canonical}>
            <span class="journal-conflict-keep"> · canonical</span>
          </Show>
        </button>
        <span class="journal-conflict-actions">
          <button class="settings-btn" title="Open this exact file (editable)" onClick={props.onOpen}>
            Open
          </button>
          <Show when={props.onMerge}>
            <button class="settings-btn" title="Append this file's blocks to the canonical day, then trash it" onClick={props.onMerge}>
              Merge
            </button>
          </Show>
          <button class="settings-btn" title="Move this file to a uniquely-named page" onClick={() => { setRenaming(true); setNewName(""); }}>
            Rename…
          </button>
          <button class="settings-btn settings-btn-danger" onClick={props.onTrash}>
            Trash
          </button>
        </span>
      </div>
      <div class="journal-conflict-preview">{props.file.preview}</div>
      <Show when={renaming()}>
        <div class="journal-conflict-rename">
          <input
            class="settings-input"
            placeholder="New page name"
            value={newName()}
            onInput={(e) => setNewName(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submitRename();
              else if (e.key === "Escape") setRenaming(false);
            }}
          />
          <button class="settings-btn" onClick={submitRename}>Save</button>
          <button class="settings-btn" onClick={() => setRenaming(false)}>Cancel</button>
        </div>
      </Show>
      <Show when={open()}>
        <pre class="journal-conflict-content">{content.loading ? "…" : content() || "(empty file)"}</pre>
      </Show>
    </>
  );
}

// Duplicate journal days: a date that resolves to >1 file (e.g. a date-stem file
// plus a title-named one, usually from a date-format change). Tine never
// auto-merges, so list each file with reconcile actions (Open/Merge/Rename/Trash).
function JournalConflictsPanel(): JSX.Element {
  void refreshJournalConflicts(); // refresh when the Backups tab opens
  // Run a reconcile op, toast the outcome, and refresh the (now-changed) list.
  const reconcile = async (op: () => Promise<void>, ok: string) => {
    try {
      await op();
      pushToast(ok, "success");
      await refreshJournalConflicts(true);
    } catch (e) {
      pushToast(`Couldn’t do that: ${String(e)}`, "error");
    }
  };
  const trashFile = async (name: string) => {
    if (
      !(await backend().confirm(
        `Move the journal file “${name}” to the trash?\n\n` +
          `It's a duplicate of another file for the same day. It moves to logseq/.tine-trash (recoverable).`
      ))
    )
      return;
    await reconcile(() => backend().trashJournalFile(name), `Moved ${name} to trash`);
  };
  const openFileRow = (file: JournalFile, title: string) => {
    openFile(file.path, title, "journal");
    closeSettings();
  };
  const openDay = (title: string) => {
    openPage(title, "journal");
    closeSettings();
  };
  return (
    <Show when={journalConflicts().length}>
      <div class="settings-section" style={{ "margin-top": "18px" }}>
        Duplicate journal days
      </div>
      <div class="settings-hint settings-block">
        These days have more than one file (e.g. a <code>2026_06_26.org</code> and a
        title-named <code>Friday, 26-06-2026.org</code>) — usually left over from changing the
        date format. <strong>Open</strong> reaches a file directly (it's editable and saves back
        to itself); <strong>Merge</strong> folds a stray into the canonical day;{" "}
        <strong>Rename</strong> turns it into a normal page; <strong>Trash</strong> removes the
        redundant one (recoverable).
      </div>
      <For each={journalConflicts()}>
        {(c) => {
          const canonical = c.files.find((f) => f.canonical);
          return (
            <div class="settings-block">
              <button class="settings-asset-name" onClick={() => openDay(c.title)}>
                {c.title} →
              </button>
              <For each={c.files}>
                {(f) => (
                  <ConflictFileRow
                    file={f}
                    onOpen={() => openFileRow(f, c.title)}
                    onMerge={!f.canonical && canonical ? () => void reconcile(
                      () => backend().mergePages(f.path, canonical.path),
                      `Merged ${f.name} into ${canonical.name}`
                    ) : undefined}
                    onRename={(n) => void reconcile(
                      () => backend().renameFileToPage(f.path, n),
                      `Renamed ${f.name} → ${n}`
                    )}
                    onTrash={() => void trashFile(f.name)}
                  />
                )}
              </For>
            </div>
          );
        }}
      </For>
    </Show>
  );
}

function AssetsTab(): JSX.Element {
  const [list, setList] = createSignal<AssetInfo[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [scanned, setScanned] = createSignal(false);
  const [trashInfo, setTrashInfo] = createSignal<TrashStats>({ count: 0, bytes: 0 });

  const fmtSize = (n: number) =>
    n >= 1 << 20 ? `${(n / (1 << 20)).toFixed(1)} MB` : n >= 1024 ? `${Math.round(n / 1024)} KB` : `${n} B`;
  const fmtDate = (secs: number | null) =>
    secs == null
      ? ""
      : new Date(secs * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
  const total = () => list().reduce((s, a) => s + a.size, 0);

  const refreshTrash = async () => {
    try {
      setTrashInfo(await backend().assetTrashStats());
    } catch {
      /* trash stats are best-effort */
    }
  };

  const refresh = async () => {
    setBusy(true);
    try {
      // Persist edits first so a just-deleted block's media counts as orphaned
      // (and a just-inserted one counts as referenced).
      await flushAll();
      setList(await backend().listOrphanAssets());
      setScanned(true);
      await refreshTrash();
    } catch (e) {
      pushToast(`Scan failed: ${String(e)}`, "error");
    } finally {
      setBusy(false);
    }
  };

  const open = async (a: AssetInfo) => {
    try {
      await backend().openAsset(a.name);
    } catch (e) {
      pushToast(`Couldn’t open ${a.name}: ${String(e)}`, "error");
    }
  };

  const trash = async (a: AssetInfo) => {
    // No confirm: the file only moves to the recoverable logseq/.tine-trash, so
    // trashing a batch stays fast. (Empty-trash, which is permanent, still asks.)
    try {
      await backend().trashAsset(a.name);
      setList((l) => l.filter((x) => x.name !== a.name));
      pushToast(`Moved ${a.name} to trash`, "success");
      await refreshTrash();
    } catch (e) {
      pushToast(`Couldn’t trash: ${String(e)}`, "error");
    }
  };

  const emptyTrash = async () => {
    const info = trashInfo();
    if (!info.count) return;
    if (
      !(await backend().confirm(
        `Permanently delete ${info.count} file${info.count === 1 ? "" : "s"} (${fmtSize(info.bytes)}) in the trash?\n\n` +
          `This cannot be undone — the files are removed from logseq/.tine-trash for good.`
      ))
    )
      return;
    try {
      const n = await backend().emptyAssetTrash();
      setTrashInfo({ count: 0, bytes: 0 });
      pushToast(`Emptied trash (${n} file${n === 1 ? "" : "s"})`, "success");
    } catch (e) {
      pushToast(`Couldn’t empty trash: ${String(e)}`, "error");
    }
  };

  return (
    <>
      <div class="settings-section" style={{ "margin-top": "18px" }}>
        Orphaned media
        <button class="settings-btn" style={{ "margin-left": "10px" }} disabled={busy()} onClick={() => void refresh()}>
          {busy() ? "Scanning…" : scanned() ? "Rescan" : "Scan for orphans"}
        </button>
        <Show when={trashInfo().count > 0}>
          <button
            class="settings-btn settings-btn-danger"
            style={{ "margin-left": "8px" }}
            onClick={() => void emptyTrash()}
            title="Permanently delete everything in logseq/.tine-trash"
          >
            Empty trash ({trashInfo().count})
          </button>
        </Show>
      </div>
      <div class="settings-hint settings-block">
        Files in <code>assets/</code> that no block links to. Deleting a block never
        deletes its media (a safety net), so unused files can accumulate — review and
        trash them here. Trashed files move to <code>logseq/.tine-trash</code> (recoverable);
        click a name to open it in your default app.
      </div>
      <Show when={scanned()}>
        <Show
          when={list().length}
          fallback={<div class="settings-hint settings-block">No orphaned media — every asset is referenced. 🎉</div>}
        >
          <div class="settings-hint settings-block">
            {list().length} orphan{list().length === 1 ? "" : "s"} · {fmtSize(total())} reclaimable
          </div>
          <div class="settings-backups">
            <For each={list()}>
              {(a) => (
                <div class="settings-asset-row">
                  <button class="settings-asset-name mono" title="Open in the default app" onClick={() => void open(a)}>
                    {a.name}
                  </button>
                  <span class="settings-asset-date">{fmtDate(a.modified)}</span>
                  <span class="settings-backup-files mono">{fmtSize(a.size)}</span>
                  <button class="settings-btn" onClick={() => void trash(a)}>
                    Trash
                  </button>
                </div>
              )}
            </For>
          </div>
        </Show>
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

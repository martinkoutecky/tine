import { codeWrapping, changeCodeWrapping } from "../codeDisplay";
import { graphConfigProblem } from "../graph";
import { ResourceFailure } from "./ResourceFailure";
import { reportUiFailure } from "../uiFailure";
import { For, Show, Suspense, createEffect, createMemo, createResource, createSignal, createUniqueId, onCleanup, onMount, type JSX } from "solid-js";
import { DiagnosticsTab } from "./DiagnosticsTab";
import { PluginsTab } from "./PluginsTab";
import { AboutTab } from "./AboutTab";
import { JournalFilenamePanel } from "./JournalFilenamePanel";
import { ConflictFileRow } from "./JournalConflictFileRow";
import { settingsOpen, closeSettings, settingsTabRequest, clearSettingsTabRequest, workflow, changeWorkflow, timetrackingEnabled, changeTimetrackingEnabled, showBrackets, changeShowBrackets, changePreferredFormat, changeJournalTitleFormat, shortcutOverrides, setShortcutOverride, resetShortcutOverride, accentColor, changeAccent, wideMode, toggleWideMode, documentMode, toggleDocumentMode, docModeEnterForNewBlock, changeDocModeEnterForNewBlock, logicalOutdenting, changeLogicalOutdenting, typographyMode, setTypographyMode, autoPairing, setAutoPairing, dimInFocus, setDimInFocus, changeStartOfWeek, carryKeepsContext, setCarryKeepsContext, carryHeader, setCarryHeader, carryDays, setCarryDays, showCarryButtons, setShowCarryButtons, agendaDaysBack, setAgendaDaysBack, agendaDaysAhead, setAgendaDaysAhead, journalConflicts, refreshJournalConflicts, refreshSyncConflicts, type SettingsTabId } from "../ui";
import { pendingConflictCount } from "../liveConflicts";
import { setJournalTemplate, graphMeta } from "../graphSession";
import { pushToast } from "../toasts";
import { interfaceZoom, zoomIn, zoomOut, zoomReset } from "../zoom";
import { smoothScrollEnabled, setSmoothScroll } from "../smoothScroll";
import { isMac, nativeFrameEnabled, setNativeFrame } from "../nativeChrome";
import {
  copyIncludeSubtree,
  setCopyIncludeSubtree,
  copyStripCollapsed,
  setCopyStripCollapsed,
  refClickZoom,
  setRefClickZoom,
} from "../copySettings";
import { navReuseTabs, setNavReuseTabs } from "../navSettings";
import { spaceAfterRefCompletion, setSpaceAfterRefCompletion } from "../refCompletionSettings";
import { allowLocalFileImages, setAllowLocalFileImages } from "../localFileSettings";
import { linkAutocompletePolicy, setLinkAutocompletePolicy, type LinkAutocompletePolicy } from "../editor/linkDefault";
import {
  spellcheckEnabled,
  setSpellcheckEnabled,
  spellcheckLanguages,
  spellcheckDictionaries,
  toggleSpellcheckLanguage,
  languageDisplayName,
  loadDictionaries,
  parseLanguages,
} from "../spellcheckSettings";
import {
  assetNameFormat,
  setAssetNameFormat,
  DEFAULT_ASSET_NAME_FORMAT,
  STAMPED_ASSET_NAME_FORMAT,
} from "../assetSettings";
import { MEDIA_EDITORS, type MediaEditor } from "../mediaEditors";
import { detectMediaEditorCommand, mediaEditorCommand, setMediaEditorCommand } from "../mediaEditorSettings";
import { formatAssetName } from "../media";
import { openConflicts, openPage, openFile } from "../router";
import { commandDefaults, eventToBindingString, setKeybindingsSuspended } from "../keybindings";
import { ShortcutsSettingsPane } from "./HelpShortcuts";
import { Field, Toggle } from "./settingsField";
import { AlwaysAskSetting } from "./AlwaysAskSetting";
import { ContentWidthFields } from "./ContentWidthFields";
import { QueryExportLimitSetting } from "./QueryExportLimitSetting";
import { GraphPublish } from "./GraphPublish";
import { HomePageSetting } from "./HomePageSetting";
import { SETTING_SEARCH, settingMatches, advancedMatch, type SettingSearchEntry } from "./settingsSearch";
import { settingsMaximized, setSettingsMaximized } from "../settingsLayout";
import { ThemeChoice } from "./ThemeChoice";
import { ThemeSettings } from "./ThemeSettings";
import { flushAll } from "../document";
import { backend, isTauri, type BackupInfo } from "../backend";
import { restoreBackupFromSettings } from "../backupRestore";
import { captureBinding, graphScopedSignal, refuseStaleWrite } from "../binding";
import type { AssetInfo, TrashStats, JournalFile } from "../types";
import { DEFAULT_TITLE_FORMAT, formatJournal, appNow } from "../journal";
import {
  launcherRankingEnabled,
  resetLauncherRanking,
  setLauncherRankingEnabled,
} from "../launcherRanking";
import { registerTransientLayer } from "../transientLayers";
import { JOURNAL_TITLE_FORMATS } from "../journalTitleFormats";
import { writePreference, loadPreference } from "../preferenceWrites";
import { bindingOwner, graphOwner, latestOwner, readOwned, writeOwned } from "../owned";
import { readOr } from "../resourceRead";
import { activeWindow, listen, queryAllWindows, queryAllWindowsAll } from "../windowRealm";
const TABS: { id: SettingsTabId; label: string }[] = [
  { id: "appearance", label: "Appearance" },
  { id: "editor", label: "Editor" },
  { id: "journals", label: "Journals" },
  { id: "files", label: "Files" },
  { id: "backups", label: "Backups & recovery" },
  { id: "graph", label: "Graph" },
  { id: "plugins", label: "Plugins" },
  { id: "diagnostics", label: "Help & diagnostics" },
  { id: "shortcuts", label: "Keyboard shortcuts" },
  { id: "about", label: "About" },
];
export function Settings(): JSX.Element {
  const [tab, setTab] = createSignal<SettingsTabId>("appearance");
  const [settingsQuery, setSettingsQuery] = createSignal("");
  const matches = createMemo(() => {
    const query = settingsQuery();
    return query.trim() ? SETTING_SEARCH.filter((entry) => settingMatches(entry, query)) : [];
  });
  const openSearchResult = (entry: SettingSearchEntry) => {
    setTab(entry.tab);
    queueMicrotask(() => {
      const fields = queryAllWindowsAll<HTMLElement>("[data-setting-label]");
      fields.find((field) => field.dataset.settingLabel === entry.label)?.scrollIntoView({ block: "center" });
    });
  };

    const shortcuts = () => {
    const cfg = graphMeta()?.shortcuts ?? {};
    const ov = shortcutOverrides();
    return commandDefaults().map((c) => ({
      ...c,
      effective: ov[c.id] ?? cfg[c.id] ?? c.binding,
      overridden: c.id in ov,
    }));
  };

  createEffect(() => {
    if (!settingsOpen()) return;
    const requested = settingsTabRequest();
    if (!requested) return;
    setTab(requested);
    clearSettingsTabRequest();
  });

  // Recording: capture the next chord for the command being remapped.
  const [recording, setRecording] = createSignal<string | null>(null);
  // Settings owns its semantic Escape rungs.  Registering here (rather than in
  // App) keeps shortcut recording/search/disclosures from being skipped by a
  // blanket modal close and ensures disposal follows this component lifetime.
  createEffect(() => {
    if (!settingsOpen()) return;
    const unregister = registerTransientLayer({
      id: "settings",
      root: () => queryAllWindows<HTMLElement>(".settings-modal"),
      dismiss: () => {
        if (recording()) { setRecording(null); return true; }
        if (settingsQuery()) { setSettingsQuery(""); return true; }
        closeSettings();
        return true;
      },
    });
    onCleanup(unregister);
  });
  createEffect(() => {
    const id = recording();
    if (!id) {
      setKeybindingsSuspended(false);
      return;
    }
    setKeybindingsSuspended(true);
    onCleanup(() => setKeybindingsSuspended(false));
    const onKey = (e: KeyboardEvent) => {
      // Escape belongs to the one capture dispatcher.  It will dismiss this
      // recording rung before the lower Settings/modal ladder.
      if (e.key === "Escape" || e.isComposing || e.keyCode === 229) return;
      e.preventDefault();
      e.stopPropagation();
      const b = eventToBindingString(e);
      if (!b) return; // bare modifier — keep waiting
      setShortcutOverride(id, b);
      setRecording(null);
    };
    onCleanup(listen(activeWindow(), "keydown", onKey, true));
  });

  return (
    <Show when={settingsOpen()}>
      <div class="modal-overlay" classList={{ "settings-maximized": settingsMaximized() }} onClick={closeSettings}>
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
              <input
                class="settings-search-input"
                type="search"
                placeholder={tab() === "shortcuts" ? "Search shortcuts…" : "Search settings…"}
                aria-label={tab() === "shortcuts" ? "Search shortcuts" : "Search settings"}
                value={settingsQuery()}
                onInput={(event) => setSettingsQuery(event.currentTarget.value)}
                onKeyDown={(event) => {
                  if (event.isComposing || event.keyCode === 229) return;
                  if (event.key === "Escape" && settingsQuery()) {
                    event.preventDefault();
                    setSettingsQuery("");
                  }
                }}
              />
              <button class="icon-btn settings-maximize" type="button"
                aria-label={settingsMaximized() ? "Restore settings size" : "Maximize settings"}
                aria-pressed={settingsMaximized()}
                onClick={() => setSettingsMaximized(!settingsMaximized())}>
                {settingsMaximized() ? "❐" : "□"}
              </button>
              <button class="icon-btn" onClick={closeSettings}>
                ✕
              </button>
            </div>
            <div class="settings-pane-body">
              <Show when={graphConfigProblem()}><p role="alert">config.edn could not be read. This graph is read-only. Repair the config and reopen the graph.</p></Show>
              <Suspense fallback={<div class="settings-pane-pending" aria-hidden="true" />}>
              <Show when={settingsQuery().trim() && tab() !== "shortcuts"}>
                <div class="settings-search-results" aria-live="polite">
                  <Show when={matches().length} fallback={<div class="settings-search-empty">No matching settings</div>}>
                    <For each={matches()}>
                      {(entry) => (
                        <button type="button" class="settings-search-result" onClick={() => openSearchResult(entry)}>
                          <span>{entry.label}</span>
                          <small>{TABS.find((candidate) => candidate.id === entry.tab)?.label}{entry.level === "advanced" ? " › Advanced" : ""}</small>
                        </button>
                      )}
                    </For>
                  </Show>
                </div>
              </Show>
              <Show when={tab() === "appearance"}>
                <AppearanceTab search={settingsQuery()} />
              </Show>
              <Show when={tab() === "editor"}>
                <EditorTab search={settingsQuery()} />
              </Show>
              <Show when={tab() === "journals"}>
                <JournalsTab search={settingsQuery()} />
              </Show>
              <Show when={tab() === "files"}>
                <FilesTab search={settingsQuery()} />
              </Show>
              <Show when={tab() === "backups"}>
                <BackupsTab />
              </Show>
              <Show when={tab() === "graph"}>
                <HomePageSetting />
                <GraphPublish /><QueryExportLimitSetting />
              </Show>
              <Show when={tab() === "plugins"}>
                <PluginsTab />
              </Show>
              <Show when={tab() === "diagnostics"}>
                <DiagnosticsTab />
              </Show>
              <Show when={tab() === "shortcuts"}>
                <ShortcutsSettingsPane
                  shortcuts={shortcuts()}
                  search={settingsQuery()}
                  recording={recording()}
                  onRecord={(id) => setRecording(recording() === id ? null : id)}
                  onUnbind={(id) => { setRecording(null); setShortcutOverride(id, "false"); }}
                  onReset={resetShortcutOverride}
                />
              </Show>
              <Show when={tab() === "about"}>
                <AboutTab />
              </Show>
              </Suspense>
            </div>
          </div>
        </div>
      </div>
    </Show>
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
    <div class="settings-field og-field" data-setting-label={props.label} classList={{ "og-diverges": diverges() }}>
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

function AdvancedSection(props: { tab: SettingsTabId; forceOpen: boolean; children: JSX.Element }): JSX.Element {
  const layerId = `settings-advanced-${createUniqueId()}`;
  const key = `tine.settings.advanced.${props.tab}`;
  let initial = false;
  try { initial = localStorage.getItem(key) === "1"; }
  catch { pushToast("Could not load Advanced section preference.", "error"); }
  const [open, setOpen] = createSignal(initial);
  let button: HTMLButtonElement | undefined;
  const expanded = () => props.forceOpen || open();
  const persist = (value: boolean) => {
    try { localStorage.setItem(key, value ? "1" : "0"); }
    catch { pushToast("Could not save Advanced section preference.", "error"); }
  };
  const toggle = () => {
    const next = !open();
    setOpen(next);
    persist(next);
  };
  createEffect(() => {
    if (!open() || props.forceOpen) return;
    const unregister = registerTransientLayer({
      id: layerId,
      parentId: "settings",
      root: () => button?.closest<HTMLElement>(".settings-advanced") ?? null,
      trigger: () => button ?? null,
      dismiss: () => { setOpen(false); persist(false); return true; },
    });
    onCleanup(unregister);
  });
  return (
    <section class="settings-advanced">
      <button
        ref={button}
        type="button"
        class="settings-advanced-toggle"
        aria-expanded={expanded()}
        onClick={toggle}
        onKeyDown={(event) => {
          if (event.isComposing || event.keyCode === 229) return;
          if (event.key === "Escape" && open() && !props.forceOpen) {
            event.preventDefault();
            setOpen(false);
            persist(false);
            queueMicrotask(() => button?.focus());
          }
        }}
      >
        <span aria-hidden="true">{expanded() ? "▾" : "▸"}</span> Advanced
      </button>
      <Show when={expanded()}>
        <div class="settings-advanced-body">{props.children}</div>
      </Show>
    </section>
  );
}

function AppearanceTab(props: { search: string }): JSX.Element {
  const [savingNativeFrame, setSavingNativeFrame] = createSignal(false);
  const changeNativeFrame = async () => {
    if (savingNativeFrame()) return;
    setSavingNativeFrame(true);
    const next = !nativeFrameEnabled();
    try {
      await setNativeFrame(next);
      pushToast("Saved. Restart Tine to apply the window-frame change.", "info");
    } catch (error) {
      pushToast(`Couldn't save the window-frame setting. (${String(error)})`, "error");
    } finally {
      setSavingNativeFrame(false);
    }
  };

  return (
    <>
      <div class="settings-row">
        <span class="settings-label">Mode</span>
        <ThemeChoice />
      </div>

      <ThemeSettings />

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
        label="Document-mode Enter creates a new block"
        hint={<>Keep the normal Enter = new block and Shift + Enter = line break mapping while Document mode is on. Off (the default) swaps them, like Logseq. Saved to <code>:shortcut/doc-mode-enter-for-new-block?</code> in <code>config.edn</code>.</>}
      >
        <Toggle
          on={docModeEnterForNewBlock()}
          onClick={() => changeDocModeEnterForNewBlock(!docModeEnterForNewBlock())}
        />
      </Field>
      <Field
        label="Show brackets"
        hint={<>Show the <code>[[ ]]</code> around page references. Saved to <code>:ui/show-brackets?</code> in <code>config.edn</code>; toggle with <code>mod+c mod+b</code>.</>}
      >
        <Toggle on={showBrackets()} onClick={() => changeShowBrackets(!showBrackets())} />
      </Field>
      <Field
        label="Typographic replacements"
        hint="Show arrows and dashes as glyphs — `->` → →, `-->` → ⟶, `--` → – (en dash), `---` → — (em dash). “While reading” keeps your Markdown as ASCII and only changes the rendered view (like `\Delta` → Δ); “While typing” rewrites the source itself as you type. A Tine touch, not Logseq."
      >
        <select
          class="settings-select"
          value={typographyMode()}
          onChange={(e) => {
            const v = e.currentTarget.value;
            setTypographyMode(v === "off" ? "off" : v === "type" ? "type" : "render");
          }}
        >
          <option value="render">On (while reading)</option>
          <option value="type">On (while typing)</option>
          <option value="off">Off</option>
        </select>
      </Field>

      <Field label="Wrap code lines" hint="Wrap long code lines in every code block while reading and editing. Off by default, as in Logseq. Saved on this device.">
        <Toggle on={codeWrapping()} onClick={() => changeCodeWrapping(!codeWrapping())} />
      </Field>
      <Field
        label="Auto-pair brackets & quotes"
        hint="Typing ( [ { &quot; ` inserts the matching closer with the caret between, wraps a selection, types through a closer, and Backspace on an empty pair clears both. (Page-ref `[[ ]]` always auto-closes.) On by default, as in Logseq."
      >
        <Toggle on={autoPairing()} onClick={() => setAutoPairing(!autoPairing())} />
      </Field>

      <OgField
        label="Space after inserting a reference"
        hint="After you pick a page from [[…]] or a block from ((…)) autocomplete, the caret lands past the closing brackets. ON (Tine default) also drops a space there so the next word flows on without stepping over the brackets; a block-final space is trimmed on save, so it never persists."
        ogNote="no space — the caret sits right after the closing brackets."
        ogValue={false}
        on={spaceAfterRefCompletion()}
        onToggle={() => setSpaceAfterRefCompletion(!spaceAfterRefCompletion())}
      />

      <Field
        label="Dim in focus mode"
        hint="Auto-enable dim inactive blocks (t b) when entering focus mode (t f)."
      >
        <Toggle on={dimInFocus()} onClick={() => setDimInFocus(!dimInFocus())} />
      </Field>

      <Field
        label="Load local-file images"
        hint="Let raw-HTML <img> tags in notes load images from absolute paths anywhere on this computer (e.g. an imported note's <img src=&quot;/home/…/pic.png&quot;>). Off by default — this is a permission: a synced or imported note isn't self-authored, so only enable it for graphs you trust. In-graph images and https images always work regardless."
      >
        <Toggle on={allowLocalFileImages()} onClick={() => setAllowLocalFileImages(!allowLocalFileImages())} />
      </Field>

      <AdvancedSection tab="appearance" forceOpen={advancedMatch("appearance", props.search)}>
        <ContentWidthFields />
        <Field
          label="Smooth scrolling (experimental)"
          hint="Animate the journal feed's scrolling to smooth out WebKitGTK's stepped mouse-wheel jumps. Off by default; this is a feel experiment — turn it off if it gets in the way."
        >
          <Toggle on={smoothScrollEnabled()} onClick={() => setSmoothScroll(!smoothScrollEnabled())} />
        </Field>
      </AdvancedSection>

      {/* Window chrome. macOS always uses its native frame (rounded corners +
          traffic lights, via the build-time Overlay title bar), so the toggle is
          only meaningful — and only shown — on Linux/Windows. */}
      <Show when={isTauri() && !isMac}>
        <Field
          label="System title bar & window controls"
          hint="Use your OS's native window frame (title bar, minimize/maximize/close, rounded corners) instead of Tine's compact built-in controls. Restart Tine after changing this setting. Off by default — the built-in controls save a row of vertical space."
        >
          <Toggle on={nativeFrameEnabled()} onClick={() => void changeNativeFrame()} />
        </Field>
      </Show>
      <Show when={isTauri() && isMac}>
        <Field
          label="Window controls"
          hint="macOS draws Tine with native rounded corners and traffic-light buttons."
        >
          <span style={{ color: "var(--text-muted)", "font-size": "12px" }}>Native (macOS)</span>
        </Field>
      </Show>

    </>
  );
}

function JournalTemplateField(): JSX.Element {
  let alive = true; onCleanup(() => { alive = false; });
  const [templatesResource, { refetch }] = createResource(async () => {
    const owner = graphOwner(() => alive);
    try { const result = await readOwned(owner, backend().listTemplates()); return result.kind === "current" ? result.value : undefined; }
    catch (error) { if (owner()) reportUiFailure("template-read", error); throw error; }
  });
  const current = () => graphMeta()?.default_journal_template ?? "";
  const list = () => readOr(templatesResource, undefined, "journal templates") ?? [];
  const selected = () => list().find((t) => t.name === current());
  const missing = () => templatesResource.error === undefined && !templatesResource.loading && current() !== "" && !list().some((t) => t.name === current());
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
        <ResourceFailure of={templatesResource} what="journal templates" onRetry={() => void refetch()} />
        <select
          class="settings-select"
          value={current()}
          disabled={templatesResource.error !== undefined || templatesResource.loading}
          onChange={(e) => setJournalTemplate(e.currentTarget.value || null)}
        >
          <option value="">(none) — blank days</option>
          <Show when={missing() || templatesResource.error !== undefined}>
            <option value={current()}>{current()}{templatesResource.error === undefined ? " (not found)" : " (unavailable)"}</option>
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
  const today = appNow();
  const current = () => graphMeta()?.journal_page_title_format || DEFAULT_TITLE_FORMAT;
  const options = () => [current(), ...JOURNAL_TITLE_FORMATS.filter((format) => format !== current())];
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

function EditorTab(props: { search: string }): JSX.Element {
  // Re-scan installed dictionaries each time Settings opens (the user may have
  // just installed one). The rows are the union of installed ∪ already-selected,
  // so a selected-but-uninstalled language still shows (flagged) instead of
  // silently vanishing.
  onMount(() => void loadDictionaries());
  const dictRows = createMemo(() => {
    const installed = new Set(spellcheckDictionaries());
    const selected = new Set(parseLanguages(spellcheckLanguages()));
    const codes = [...new Set([...installed, ...selected])].sort((a, b) =>
      languageDisplayName(a).localeCompare(languageDisplayName(b)),
    );
    return codes.map((code) => ({
      code,
      name: languageDisplayName(code),
      selected: selected.has(code),
      installed: installed.has(code),
    }));
  });
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
        label="Spell checker"
        hint={
          <>
            Underline misspelled words while editing, with right-click suggestions and
            “add to dictionary” (uses the system spell checker). <strong>On by default</strong>,
            like Logseq. Applies live — no restart needed.
          </>
        }
      >
        <Toggle on={spellcheckEnabled()} onClick={() => setSpellcheckEnabled(!spellcheckEnabled())} />
      </Field>

      <Field
        label="Logical outdenting"
        hint={<>Move an outdented block after its parent while leaving following siblings in place. Off (the default) reparents those siblings beneath the moved block. Saved to <code>:editor/logical-outdenting?</code> in <code>config.edn</code>.</>}
      >
        <Toggle on={logicalOutdenting()} onClick={() => changeLogicalOutdenting(!logicalOutdenting())} />
      </Field>

      <Show when={spellcheckEnabled()}>
        <Field
          label="Spellcheck languages"
          hint={
            <>
              Tick the dictionaries to check — several at once is fine (e.g. English + Czech),
              and a word valid in <em>any</em> ticked language isn’t flagged, so bilingual notes
              don’t squiggle. <strong>None ticked → follows your OS locale</strong> (all Logseq
              can do). Dictionaries are discovered from the system; install more with your
              package manager (e.g. <code>hunspell-cs</code>), then Rescan.
            </>
          }
        >
          <div class="spellcheck-dicts">
            <For each={dictRows()}>
              {(row) => (
                <label class="spellcheck-dict">
                  <input
                    type="checkbox"
                    checked={row.selected}
                    onChange={(e) => toggleSpellcheckLanguage(row.code, e.currentTarget.checked)}
                  />
                  <span class="spellcheck-dict-name">{row.name}</span>
                  <code>{row.code}</code>
                  <Show when={!row.installed}>
                    <span class="spellcheck-dict-missing">not installed</span>
                  </Show>
                </label>
              )}
            </For>
            <Show when={dictRows().length === 0}>
              <div class="spellcheck-empty">
                No dictionaries found. Install one (e.g. <code>hunspell-en-us</code>), then Rescan.
              </div>
            </Show>
            <button class="spellcheck-rescan" type="button" onClick={() => void loadDictionaries()}>
              ↻ Rescan
            </button>
          </div>
        </Field>
      </Show>

      <AdvancedSection tab="editor" forceOpen={advancedMatch("editor", props.search)}>
        <Field
          label="Link autocomplete default"
          hint="Controls Enter for non-exact [[name and #name completion. OG adaptive (default) picks the shortest lexical strict-prefix match and puts Create immediately after it; fuzzy-only matches leave Create first. Prefer existing always leads with a match; Prefer exactly what I typed leads with Create. Exact existing names always select the existing page."
        >
          <select
            aria-label="Link autocomplete default"
            value={linkAutocompletePolicy()}
            onChange={(event) => setLinkAutocompletePolicy(event.currentTarget.value as LinkAutocompletePolicy)}
          >
            <option value="adaptive">OG adaptive</option>
            <option value="existing">Prefer existing</option>
            <option value="typed">Prefer exactly what I typed</option>
          </select>
        </Field>

        <Field
          label="Switch to an already-open tab when navigating"
          hint="Plain navigation to a page, journal, or exact zoomed/file-pinned view focuses the matching tab if one is already open. Middle-click and explicit Open in new tab still create another tab."
        >
          <Toggle on={navReuseTabs()} onClick={() => setNavReuseTabs(!navReuseTabs())} />
        </Field>

        <Field
          label="Learn Ctrl+K choices"
          hint="After you deliberately open the same result more than once for a query, Ctrl+K may prefer it only among equally strong matches. History stays on this device and in this graph; saved searches and queries remain deterministic."
        >
          <>
            <Toggle
              on={launcherRankingEnabled()}
              onClick={() => setLauncherRankingEnabled(!launcherRankingEnabled())}
            />
            <button
              type="button"
              class="og-revert"
              onClick={() => {
                resetLauncherRanking(graphMeta()?.root ?? "");
                pushToast("Ctrl+K ranking reset for this graph");
              }}
            >
              Reset ranking
            </button>
          </>
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
      </AdvancedSection>

      <OgField
        label="Click a block reference to zoom in"
        hint="Plain-clicking an inline ((block reference)): ON zooms into the referenced block (opens it as its own page, like Logseq); OFF (Tine default) scrolls to it in place and flashes it. Shift-click always opens it in the sidebar."
        ogNote="zooms into the referenced block on click."
        ogValue={true}
        on={refClickZoom()}
        onToggle={() => setRefClickZoom(!refClickZoom())}
      />
    </>
  );
}

function JournalsTab(props: { search: string }): JSX.Element {
  // Quick-capture Enter behaviour (app-level setting, read by the capture window).
  const [captureEnterFiles, setCaptureEnterFiles] = createSignal(false);
  let captureChanged = false;
  let captureAlive = true;
  onCleanup(() => { captureAlive = false; });
  loadPreference(captureEnterFiles, setCaptureEnterFiles, () => backend().getCaptureEnterFiles(),
    (value) => value, "capture Enter preference", () => captureAlive && !captureChanged);
  const toggleCaptureEnter = () => {
    captureChanged = true;
    const v = !captureEnterFiles();
    writePreference(captureEnterFiles, setCaptureEnterFiles, v,
      (next) => backend().setCaptureEnterFiles(next), "capture Enter preference");
  };
  return (
    <>
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

      <Field
        label="Time tracking"
        hint={
          <>
            Marker transitions write OG-compatible <code>:LOGBOOK:</code> CLOCK rows. Saved to{" "}
            <code>:feature/enable-timetracking?</code>; seconds mode follows{" "}
            <code>:logbook/settings</code> and is {graphMeta()?.logbook_with_second_support ?? true ? "on" : "off"}.
          </>
        }
      >
        <Toggle on={timetrackingEnabled()} onClick={() => changeTimetrackingEnabled(!timetrackingEnabled())} />
      </Field>

      <JournalTemplateField />

      <AdvancedSection tab="journals" forceOpen={advancedMatch("journals", props.search)}>
        <Field
          label="Quick-capture Enter key"
          hint={`In the quick-capture window: ON → Enter files the capture. OFF → Enter starts a new block; the “Quick-capture: file to today’s journal” shortcut files (default Ctrl+Shift+Enter, remappable under Keyboard shortcuts). Ctrl+Enter stays free for cycling the task marker.`}
        >
          <Toggle on={captureEnterFiles()} onClick={toggleCaptureEnter} />
        </Field>
      </AdvancedSection>

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

function BackupsTab(): JSX.Element {
  let alive = true;
  onCleanup(() => { alive = false; });
  const refreshScope = {};
  const [keep, setKeep] = createSignal(12);
  const [list, setList] = createSignal<BackupInfo[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [loading, setLoading] = createSignal(true);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const ready = () => !loading() && !loadError();

  const refresh = async () => {
    const owner = latestOwner(refreshScope, "backups", graphOwner(() => alive));
    setLoading(true);
    setLoadError(null);
    try {
      const result = await readOwned(owner, Promise.all([
        backend().getBackupKeep(),
        backend().listBackups(),
      ]));
      if (result.kind === "stale") return;
      const [nextKeep, nextList] = result.value;
      setKeep(nextKeep);
      setList(nextList);
    } catch (e) {
      if (owner()) {
        setList([]);
        setLoadError(String(e));
      }
    } finally {
      if (owner()) setLoading(false);
    }
  };

  // Load the current keep count + snapshot list when this tab mounts, before
  // enabling controls that depend on that data.
  createEffect(() => {
    void refresh();
  });
  const saveKeep = async (n: number) => {
    const v = Math.max(1, Math.min(1000, Math.floor(n) || 12));
    setKeep(v);
    const owner = bindingOwner(() => alive);
    try {
      const result = await writeOwned(owner, backend().setBackupKeep(v));
      if (result.kind === "stale") return;
      void refresh(); // a lower cap prunes immediately on the Rust side
    } catch (e) {
      pushToast(`Couldn't save: ${String(e)}`, "error");
    }
  };

  const restore = (b: BackupInfo) => {
    if (!ready() || busy()) return;
    void restoreBackupFromSettings(b, fmtStamp(b.stamp), setBusy, () => { void refresh(); });
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
          disabled={!ready() || busy()}
          onChange={(e) => void saveKeep(Number(e.currentTarget.value))}
        />
      </Field>
      <AlwaysAskSetting />

      <div class="settings-section">
        Available snapshots
        <button
          class="settings-btn"
          style={{ "margin-left": "10px" }}
          disabled={loading() || busy()}
          onClick={() => void refresh()}
        >
          Refresh
        </button>
      </div>
      <Show when={loading()}>
        <div class="settings-hint settings-block" role="status">
          Loading snapshot settings…
        </div>
      </Show>
      <Show when={loadError()}>
        {(error) => (
          <div class="settings-hint settings-block" role="alert">
            Couldn&apos;t load backup settings: {error()}
            <button class="settings-btn" style={{ "margin-left": "10px" }} onClick={() => void refresh()}>
              Retry
            </button>
          </div>
        )}
      </Show>
      <Show
        when={ready() && list().length}
        fallback={
          <Show when={ready()}>
            <div class="settings-hint settings-block">No snapshots yet.</div>
          </Show>
        }
      >
        <div class="settings-backups">
          <For each={list()}>
            {(b) => (
              <div class="settings-backup-row">
                <span class="settings-backup-when">{fmtStamp(b.stamp)}</span>
                <span class="settings-backup-files mono">{b.files} files</span>
                <button class="settings-btn" disabled={!ready() || busy()} onClick={() => void restore(b)}>
                  Restore
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>

      <JournalConflictsPanel />
      <JournalFilenamePanel />
      <ConflictOverviewPointer />
    </>
  );
}

// Orphaned-media cleanup: scan (on demand — it parses the whole graph) for
// assets/ files no block links to, and let the user move them to the recoverable
// trash. Tine never auto-deletes media (a deleted block keeps its files), so this
// is how unused media gets cleaned up.
// Duplicate journal days expose per-file reconcile actions.
function JournalConflictsPanel(): JSX.Element {
  void refreshJournalConflicts(); // refresh when the Backups tab opens
  const reconcile = async (op: () => Promise<void>, ok: string) => {
    const owner = bindingOwner();
    try {
      const result = await writeOwned(owner, op());
      if (result.kind === "stale") return;
      pushToast(ok, "success");
      await refreshJournalConflicts();
    } catch (e) {
      pushToast(`Couldn’t do that: ${String(e)}`, "error");
    }
  };
  const trashFile = async (name: string) => {
    const owner = bindingOwner();
    const confirmed = await readOwned(owner, backend().confirm(
        `Move the journal file “${name}” to the trash?\n\n` +
          `It's a duplicate of another file for the same day. It moves to logseq/.tine-trash (recoverable).`
      ));
    if (confirmed.kind === "stale" || !confirmed.value) return;
    await writeOwned(owner, reconcile(() => backend().trashJournalFile(name, "delete-page"), `Moved ${name} to trash`));
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
                      () => backend().mergePages(f.path, canonical.path, ["insert-blocks", "delete-page"]),
                      `Merged ${f.name} into ${canonical.name}`
                    ) : undefined}
                    onRename={(n) => void reconcile(
                      () => backend().renameFileToPage(f.path, n, "rename-page"),
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

// Concord inventory (og 8c): sync conflict copies, marker-bearing files and a
// copy whose page is gone live on ONE surface, the Conflicts overview, which the
// sidebar's `N conflicts` badge opens; Settings only points there. The merge
// modal that used to live here is retired: resolution happens on the page.
function ConflictOverviewPointer(): JSX.Element {
  void refreshSyncConflicts(); // refresh when the Backups tab opens
  return (
    <Show when={pendingConflictCount()}>
      <div class="settings-section" style={{ "margin-top": "18px" }}>
        Conflicts
      </div>
      <div class="settings-hint settings-block">
        {pendingConflictCount()} {pendingConflictCount() === 1 ? "item needs" : "items need"} a decision: sync conflict copies or
        version-control merge markers. The Conflicts page lists them, with{" "}
        <strong>Discard copy</strong> for sync copies; the <strong>N conflicts</strong> badge in the
        sidebar opens it too.
      </div>
      <span class="journal-conflict-actions">
        <button class="settings-btn" onClick={() => { openConflicts(); closeSettings(); }}>
          Open conflicts
        </button>
      </span>
    </Show>
  );
}

function FilesTab(props: { search: string }): JSX.Element {
  // Live preview of the asset-name template, on a fixed sample so every token is
  // visible (and the example doesn't jitter by the second). Shows both a named
  // drag/insert and a clipboard paste (which has no name → timestamp fallback).
  const sampleDate = new Date(2030, 0, 2, 3, 4, 5);
  const dragExample = () => formatAssetName(assetNameFormat(), "Holiday Photo.JPG", sampleDate);
  const pasteExample = () => formatAssetName(assetNameFormat(), undefined, sampleDate);
  // File-watch mechanism (device-local). Loaded from the backend on mount.
  const [watchMode, setWatchMode] = createSignal<"inotify" | "poll">("inotify");
  let watchChanged = false;
  let watchAlive = true;
  onCleanup(() => { watchAlive = false; });
  loadPreference(watchMode, setWatchMode, () => backend().getWatchMode(),
    (value) => value === "poll" ? "poll" : "inotify", "file watcher mode", () => watchAlive && !watchChanged);
  const changeWatchMode = (m: "inotify" | "poll") => {
    if (m === watchMode()) return;
    watchChanged = true;
    writePreference(watchMode, setWatchMode, m,
      (next) => backend().setWatchMode(next), "file watcher mode");
  };
  return (
    <>
      <div class="settings-section">Asset names</div>
      <Field
        label="New asset filename"
        hint={
          <>
            How files are named when you paste, drag, or insert media into{" "}
            <code>assets/</code>. Tokens: <code>%assetname</code> (the original file’s name),{" "}
            <code>%ext</code>, <code>%yyyymmdd</code>, <code>%hhmmss</code> — also granular{" "}
            <code>%yyyy</code> <code>%MM</code> <code>%dd</code> <code>%HH</code> <code>%mm</code>{" "}
            <code>%ss</code>. Anything else is literal. A clipboard paste has no name, so{" "}
            <code>%assetname</code> falls back to a timestamp. Device-local; Logseq keeps the
            original name for dragged files.
          </>
        }
      >
        <input
          type="text"
          class="settings-input mono"
          value={assetNameFormat()}
          spellcheck={false}
          onChange={(e) => setAssetNameFormat(e.currentTarget.value)}
        />
      </Field>
      <div class="asset-name-extras">
        <div class="settings-segment">
          <button
            classList={{ active: assetNameFormat() === DEFAULT_ASSET_NAME_FORMAT }}
            onClick={() => setAssetNameFormat(DEFAULT_ASSET_NAME_FORMAT)}
          >
            Original name
          </button>
          <button
            classList={{ active: assetNameFormat() === STAMPED_ASSET_NAME_FORMAT }}
            onClick={() => setAssetNameFormat(STAMPED_ASSET_NAME_FORMAT)}
          >
            Date + name
          </button>
        </div>
        <div class="asset-name-preview">
          <div>
            Drag <code class="mono">Holiday Photo.JPG</code> →{" "}
            <code class="mono">{dragExample()}</code>
          </div>
          <div>
            Paste an image → <code class="mono">{pasteExample()}</code>
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

      <AdvancedSection tab="files" forceOpen={advancedMatch("files", props.search)}>
        <MediaEditorsSection />
      </AdvancedSection>

      <AssetsTab />
    </>
  );
}

// Configurable external editors for diagram assets (GH #38): drawio, Excalidraw.
// Each registry entry gets one command row; empty = the OS default opener. drawio
// offers an autodetect probe.
function MediaEditorsSection(): JSX.Element {
  const [detecting, setDetecting] = createSignal<string | null>(null);
  const autodetect = async (ed: MediaEditor) => {
    setDetecting(ed.id);
    try {
      const { command, applied } = await detectMediaEditorCommand(ed);
      if (!applied) return;
      if (command) {
        pushToast(`Found: ${command}`, "success");
      } else {
        pushToast("Couldn’t find it — set the command manually.", "error");
      }
    } catch {
      pushToast("Autodetect failed.", "error");
    } finally {
      setDetecting(null);
    }
  };
  return (
    <>
      <div class="settings-section">Diagram editors</div>
      <div class="settings-hint" style={{ "margin-bottom": "8px" }}>
        Edit diagram assets in your own installed app. A <code class="mono">/drawio</code> command
        creates a new editable <code>.drawio.svg</code>; hovering any matching image shows an
        “Edit in …” button. Leave a command blank to use the system default opener. A{" "}
        <code class="mono">{"{}"}</code> in the command is replaced by the file path (otherwise it’s
        appended). Wrap a program or argument containing spaces in double quotes. Desktop only;
        device-local.
      </div>
      <For each={MEDIA_EDITORS}>
        {(ed) => (
          <Field label={ed.settingLabel}>
            <div class="media-editor-row">
              <input
                type="text"
                class="settings-input mono"
                placeholder="system default opener"
                value={mediaEditorCommand(ed.settingKey)}
                spellcheck={false}
                onChange={(e) => setMediaEditorCommand(ed.settingKey, e.currentTarget.value)}
              />
              <Show when={ed.detectable}>
                <button
                  class="settings-btn"
                  disabled={detecting() === ed.id}
                  onClick={() => void autodetect(ed)}
                >
                  {detecting() === ed.id ? "Detecting…" : "Autodetect"}
                </button>
              </Show>
            </div>
          </Field>
        )}
      </For>
    </>
  );
}

/** One graph's orphan scan (I-20): null once its binding is stale, so Trash cannot carry an old graph's name into the new one. */
const [orphanScan, setOrphanScan] = graphScopedSignal<AssetInfo[]>();

function AssetsTab(): JSX.Element {
  let alive = true; onCleanup(() => { alive = false; setOrphanScan(null); });
  const requests = {}; const list = () => orphanScan() ?? [];
  const [busy, setBusy] = createSignal(false);
  const scanned = () => orphanScan() !== null;
  const [trashInfo, setTrashInfo] = createSignal<TrashStats>({ count: 0, bytes: 0, pages: 0, journals: 0, conflicts: 0, other: 0 });

  const fmtSize = (n: number) => n >= 1 << 20 ? `${(n / (1 << 20)).toFixed(1)} MB` : n >= 1024 ? `${Math.round(n / 1024)} KB` : `${n} B`;
  const fmtDate = (secs: number | null) => secs == null ? "" : new Date(secs * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
  const total = () => list().reduce((s, a) => s + a.size, 0);
  const protectedTrashCount = () =>
    trashInfo().pages + trashInfo().journals + trashInfo().conflicts + trashInfo().other;
  const protectedTrashLabel = () =>
    [
      trashInfo().pages ? `${trashInfo().pages} page${trashInfo().pages === 1 ? "" : "s"}` : "",
      trashInfo().journals ? `${trashInfo().journals} journal${trashInfo().journals === 1 ? "" : "s"}` : "",
      trashInfo().conflicts ? `${trashInfo().conflicts} conflict${trashInfo().conflicts === 1 ? "" : "s"}` : "",
      trashInfo().other ? `${trashInfo().other} other` : "",
    ]
      .filter(Boolean)
      .join(", ");

  const refreshTrash = async () => {
    const owner = latestOwner(requests, "trash-stats", graphOwner(() => alive));
    try {
      const info = await readOwned(owner, backend().assetTrashStats());
      if (info.kind === "current") setTrashInfo(info.value);
    } catch (error) { if (owner()) reportUiFailure("trash-inventory", error); }
  };

  const refresh = async () => {
    const owner = latestOwner(requests, "scan", graphOwner(() => alive));
    setBusy(true);
    try {
      // Persist edits first so a just-deleted block's media counts as orphaned
      // (and a just-inserted one counts as referenced).
      if (!(await flushAll())) {
        if (!owner()) return;
        pushToast("Some pages couldn't be saved — resolve conflicts before scanning assets.", "error");
        return;
      }
      const assets = await readOwned(owner, backend().listOrphanAssets());
      if (assets.kind === "stale") return;
      setOrphanScan(assets.value);
      await refreshTrash();
    } catch (e) {
      if (owner()) reportUiFailure("asset-inventory", e);
    } finally {
      if (owner()) setBusy(false);
    }
  };

  const open = async (a: AssetInfo) => {
    const binding = captureBinding();
    const owner = graphOwner();
    try {
      await readOwned(owner, backend().openAsset(a.name, binding.backendGeneration));
    } catch (e) {
      pushToast(`Couldn’t open ${a.name}: ${String(e)}`, "error");
    }
  };
  const trash = async (a: AssetInfo) => {
    if (orphanScan() === null) return refuseStaleWrite("Moving that asset to the trash");
    const binding = captureBinding();
    const owner = bindingOwner();
    // Recoverable trash moves directly; emptying trash still asks.
    try {
      const result = await writeOwned(owner, backend().trashAsset(a.name, binding.backendGeneration));
      if (result.kind === "stale") return;
      if (result.value === "referenced") {
        // A page started using the file after the scan (typed outcome, GH #623): keep it.
        if (alive) setOrphanScan(list().filter((x) => x.name !== a.name));
        pushToast(`${a.name} is used by a page now, so it was kept. Scan again to refresh the list.`, "info");
        return;
      }
      if (alive) setOrphanScan(list().filter((x) => x.name !== a.name));
      pushToast(`Moved ${a.name} to trash`, "success");
      await refreshTrash();
    } catch (e) {
      pushToast(`Couldn’t trash: ${String(e)}`, "error");
    }
  };
  const emptyTrash = async () => {
    const binding = captureBinding();
    const owner = bindingOwner();
    const info = trashInfo();
    if (!info.count) return;
    const confirmed = await readOwned(owner, backend().confirm(
        `Permanently delete ${info.count} asset file${info.count === 1 ? "" : "s"} (${fmtSize(info.bytes)}) in the trash?\n\n` +
          `This cannot be undone. Page, journal, and conflict recovery files in logseq/.tine-trash will be kept.`
      ));
    if (confirmed.kind === "stale" || !confirmed.value) return;
    try {
      const result = await writeOwned(owner, backend().emptyAssetTrash(binding.backendGeneration));
      if (result.kind === "stale") return;
      const n = result.value;
      setTrashInfo((t) => ({ ...t, count: 0, bytes: 0 }));
      pushToast(`Emptied asset trash (${n} file${n === 1 ? "" : "s"})`, "success");
    } catch (e) {
      pushToast(`Couldn’t empty trash: ${String(e)}`, "error");
    }
  };
  return (
    <>
      <div class="settings-section">
        Orphaned media
        <button class="settings-btn" style={{ "margin-left": "10px" }} disabled={busy()} onClick={() => void refresh()}>
          {busy() ? "Scanning…" : scanned() ? "Rescan" : "Scan for orphans"}
        </button>
        <Show when={trashInfo().count > 0}>
          <button
            class="settings-btn settings-btn-danger"
            style={{ "margin-left": "8px" }}
            onClick={() => void emptyTrash()}
            title="Permanently delete asset files in logseq/.tine-trash"
          >
            Empty asset trash ({trashInfo().count})
          </button>
        </Show>
      </div>
      <Show when={protectedTrashCount() > 0}>
        <div class="settings-hint settings-block">
          Protected recovery trash kept: {protectedTrashLabel()}
        </div>
      </Show>
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

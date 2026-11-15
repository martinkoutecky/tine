import { settings } from "./settingDefinitions";
import { Rich } from "../i18n/Rich";
import { codeWrapping, changeCodeWrapping } from "../codeDisplay";
import { stickyParents, changeStickyParents } from "../stickyParents";
import { graphConfigUnreadable } from "../graphConfig";
import { JournalTemplateField } from "./JournalTemplateField";
import { For, Show, Suspense, createEffect, createMemo, createResource, createSignal, createUniqueId, onCleanup, onMount, type JSX } from "solid-js";
import { DiagnosticsTab } from "./DiagnosticsTab";
import { PluginsTab } from "./PluginsTab";
import { QuickShareSettings } from "./QuickShareSettings";
import { AboutTab } from "./AboutTab";
import { platformKind } from "../platform";
import { primeQuickShareGate, quickShareOffered } from "../quickShareGate";
import { JournalFilenamePanel } from "./JournalFilenamePanel";
import { ConflictFileRow } from "./JournalConflictFileRow";
import { AnnotationVisibilitySettings } from "./AnnotationVisibilitySettings";
import { settingsOpen, closeSettings, settingsTabRequest, clearSettingsTabRequest, workflow, changeWorkflow, timetrackingEnabled, changeTimetrackingEnabled, showBrackets, changeShowBrackets, changePreferredFormat, changeJournalTitleFormat, shortcutOverrides, setShortcutOverride, resetShortcutOverride, accentColor, changeAccent, wideMode, toggleWideMode, documentMode, toggleDocumentMode, docModeEnterForNewBlock, changeDocModeEnterForNewBlock, logicalOutdenting, changeLogicalOutdenting, typographyMode, setTypographyMode, autoPairing, setAutoPairing, dimInFocus, setDimInFocus, changeStartOfWeek, carryKeepsContext, setCarryKeepsContext, carryHeader, setCarryHeader, carryDays, setCarryDays, showCarryButtons, setShowCarryButtons, agendaDaysBack, setAgendaDaysBack, agendaDaysAhead, setAgendaDaysAhead, journalConflicts, refreshJournalConflicts, refreshSyncConflicts, type SettingsTabId } from "../ui";
import { pendingConflictCount } from "../liveConflicts";
import { graphMeta } from "../graphSession";
import { changedAny } from "../changeKeys";
import { JOURNAL_SECTIONS_MARKER, markerChangeKeys } from "../tineKeys";
import { locateSectionsPages } from "../journalSections";
import { pushToast } from "../toasts";
import { interfaceZoom, zoomIn, zoomOut, zoomReset } from "../zoom";
import { smoothScrollEnabled, setSmoothScroll } from "../smoothScroll";
import { isMac, nativeFrameEnabled, setNativeFrame } from "../nativeChrome";
import { touchPrimary } from "../inputCapability";
import {
  copyStripCollapsed,
  setCopyStripCollapsed,
  refClickZoom,
  setRefClickZoom,
} from "../copySettings";
import { mobileNavigationAtBottom, navReuseTabs, searchResultsOpenInThisTab, setMobileNavigationAtBottom, setNavReuseTabs, setSearchResultsOpenInThisTab, showWorkspaces, setShowWorkspaces, showHelpButton, setShowHelpButton } from "../navSettings";
import { spaceAfterRefCompletion, setSpaceAfterRefCompletion } from "../refCompletionSettings";
import { autoEmbedVideoLinks, setAutoEmbedVideoLinks } from "../pasteSettings";
import { hoverPreviewDelaySeconds, setHoverPreviewDelaySeconds, previewsEnabled, changePreviewsEnabled } from "../hoverPreviewSettings";
import { allowLocalFileImages, setAllowLocalFileImages } from "../localFileSettings";
import { linkAutocompletePolicy, setLinkAutocompletePolicy, type LinkAutocompletePolicy } from "../editor/linkDefault";
import { cm6EditorEnabled, setCm6EditorEnabled } from "../editor/cm6Setting";
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
import { noteLayoutRtl, setNoteLayoutRtl } from "../noteLayoutSetting";
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
import { bindingAlternatives, shortcutValue } from "../shortcutBindings";
import { ShortcutsSettingsPane } from "./HelpShortcuts";
import { Field, Toggle, settingLabel, settingAttributes, settingDescription, settingHint } from "./settingsField";
import { TraySettings } from "./TraySettings";
import { AlwaysAskSetting } from "./AlwaysAskSetting";
import { ContentWidthFields } from "./ContentWidthFields";
import { QueryExportLimitSetting } from "./QueryExportLimitSetting";
import { GitAutocommitSetting } from "./GitAutocommitSetting";
import { SeenSettings } from "./SeenSettings";
import { GraphPublish } from "./GraphPublish";
import { HomePageSetting } from "./HomePageSetting";
import { LanguageSetting } from "./LanguageSetting";
import { CommentNameSetting } from "./CommentNameSetting";
import { LinkSchemeSettings } from "./LinkSchemeSettings";
import { AssetsTab } from "./AssetsSettings";
import { GraphFolderSetting } from "./GraphFolderSetting";
import { msg, render, t, type Msg } from "../i18n";
import { rankedSettingMatches, settingHighlight, advancedMatch, matchedDescription } from "./settingsSearch";
import { settingEntries } from "./settingsRegistry";
import { mediaEditorSetting, type SettingDescriptor } from "./settingsDescriptor";
import { settingsMaximized, setSettingsMaximized } from "../settingsLayout";
import { ThemeChoice } from "./ThemeChoice";
import { ThemeSettings } from "./ThemeSettings";
import { ToolbarSettings } from "./ToolbarSettings";
import { CustomCssSettings } from "./CustomCssSettings";
import { StyleKnobsSettings } from "./StyleKnobsSettings";
import { backend, isTauri, type BackupInfo } from "../backend";
import { restoreBackupFromSettings } from "../backupRestore";
import type { JournalFile } from "../types";
import { DEFAULT_TITLE_FORMAT, formatJournal, appNow } from "../journal";
import {
  launcherRankingEnabled,
  pinyinMatchingEnabled,
  setPinyinMatchingEnabled,
  resetPinyinMatchingSetting,
  resetLauncherRanking,
  setLauncherRankingEnabled,
} from "../launcherRanking";
import { registerTransientLayer } from "../transientLayers";
import { JOURNAL_TITLE_FORMATS } from "../journalTitleFormats";
import { writePreference, loadPreference } from "../preferenceWrites";
import { bindingOwner, graphOwner, latestOwner, readOwned, writeOwned } from "../owned";
import { readOr } from "../resourceRead";
import { activeWindow, listen, queryAllWindows } from "../windowRealm";
const TABS: { id: SettingsTabId; label: Msg }[] = [
  { id: "appearance", label: msg("settings-ui-appearance") },
  { id: "themes", label: msg("themes-themes") },
  { id: "editor", label: msg("settings-ui-editor") },
  { id: "journals", label: msg("settings-ui-journals") },
  { id: "files", label: msg("settings-ui-files") },
  { id: "backups", label: msg("settings-ui-backups-recovery") },
  { id: "graph", label: msg("settings-ui-graph") },
  { id: "sharing", label: msg("settings-ui-sharing") },
  { id: "plugins", label: msg("settings-ui-plugins") },
  { id: "diagnostics", label: msg("settings-ui-help-diagnostics") },
  { id: "shortcuts", label: msg("settings-ui-keyboard-shortcuts") },
  { id: "about", label: msg("settings-ui-about") },
];
/** How long a search result waits for an async-mounted target before giving up. */
const SEARCH_SCROLL_WAIT_MS = 1000;
const SETTINGS_FLASH_MS = 2000;
export function Settings(): JSX.Element {
  const phoneQuery = activeWindow().matchMedia?.("(max-width: 480px) and (pointer: coarse)");
  const [phone, setPhone] = createSignal(phoneQuery?.matches ?? false);
  const updatePhone = () => setPhone(phoneQuery?.matches ?? false);
  phoneQuery?.addEventListener("change", updatePhone);
  onCleanup(() => phoneQuery?.removeEventListener("change", updatePhone));
  const [paneOpen, setPaneOpen] = createSignal(false);
  const [tab, setTab] = createSignal<SettingsTabId>("appearance");
  const [settingsQuery, setSettingsQuery] = createSignal("");
  // A result opens its tab with the matching Advanced section revealed, even
  // though navigation ends the search and restores the normal tab body.
  const [revealedQuery, setRevealedQuery] = createSignal("");
  const [searchTarget, setSearchTarget] = createSignal<SettingDescriptor>();
  const shortcutSearch = () => tab() === "shortcuts" && (!phone() || paneOpen());
  const searching = () => !!settingsQuery().trim() && !shortcutSearch();
  let searchInput: HTMLInputElement | undefined;
  let modalRoot: HTMLDivElement | undefined;
  const [settingsPlatform] = createResource(platformKind);
  // Unknown native platforms fail closed: do not reveal a package host whose
  // platform policy could not be established (ADR 0052; master b89a9e77f).
  const pluginsAvailable = () => {
    const platform = readOr(settingsPlatform, undefined, "settings-platform");
    return platform === "desktop" || platform === "android";
  };
  // Sharing appears where quick share is on (DESIGN §5: off in shipped builds until its viewer is deployed).
  // A session-wide signal primed at graph load, not a resource: a resource read here would suspend the
  // whole dialog (GH #409), and a read per open inserted the tab after the list painted.
  const sharingAvailable = quickShareOffered;
  onMount(() => { void primeQuickShareGate(); });
  const availableTabs = createMemo(() => TABS.filter((entry) =>
    (entry.id !== "plugins" || pluginsAvailable()) && (entry.id !== "sharing" || sharingAvailable())));
  const matches = createMemo(() => {
    const query = settingsQuery();
    return query.trim() ? rankedSettingMatches(settingEntries().filter((entry) => availableTabs().some((tab) => tab.id === entry.tab) && (touchPrimary() || !entry.touchOnly)), query) : [];
  });
  // A setting whose field mounts after an async load (Developer tools waits for
  // the platform probe) is not in the DOM when the tab opens, so the lookup is
  // retried per frame for a bounded time and then given up silently (GH #706).
  let scrollRun = 0;
  onCleanup(() => { scrollRun++; });
  const openSearchResult = (entry: SettingDescriptor) => {
    setSearchTarget(entry);
    setRevealedQuery(settingsQuery());
    setSettingsQuery("");
    setTab(entry.tab);
    setPaneOpen(true);
    const run = ++scrollRun;
    const findTarget = () => Array.from(modalRoot?.querySelectorAll<HTMLElement>("[data-setting-id]") ?? [])
      .find((field) => field.dataset.settingId === entry.id);
    const deadline = Date.now() + SEARCH_SCROLL_WAIT_MS;
    const attempt = (first: boolean) => {
      if (run !== scrollRun) return;
      const target = findTarget();
      if (first) searchInput?.focus({ preventScroll: true });
      if (target) {
        target.scrollIntoView({ block: "center" });
        // Mark where the jump landed for 2 s, so the eye finds the setting (GH #706).
        const row = target.closest<HTMLElement>(".settings-field, .settings-row, .settings-section") ?? target;
        row.classList.remove("settings-jump-flash");
        void row.offsetWidth;
        row.classList.add("settings-jump-flash");
        setTimeout(() => row.classList.remove("settings-jump-flash"), SETTINGS_FLASH_MS);
      }
      else if (Date.now() < deadline) (typeof requestAnimationFrame === "function" ? requestAnimationFrame : (cb: () => void) => setTimeout(cb, 16))(() => attempt(false));
    };
    queueMicrotask(() => attempt(true));
  };

    const shortcuts = () => {
    const cfg = graphMeta()?.shortcuts ?? {};
    const ov = shortcutOverrides();
    return commandDefaults().map((c) => ({
      ...c,
      alias: c.id in ov || c.id in cfg ? undefined : c.alias,
      effective: ov[c.id] ?? cfg[c.id] ?? c.binding,
      overridden: c.id in ov,
    }));
  };

  createEffect(() => {
    if (!settingsPlatform.loading && tab() === "plugins" && !pluginsAvailable()) setTab("appearance");
  });

  createEffect(() => { if (!settingsOpen()) { setPaneOpen(false); if (phone()) setSettingsQuery(""); } });
  createEffect(() => {
    if (!settingsOpen()) return;
    const requested = settingsTabRequest();
    if (!requested) return;
    if (requested === "plugins" && settingsPlatform.loading) return;
    if (requested === "toolbar") openSearchResult(settings["toolbar-title"]);
    else {
      setTab(requested === "plugins" && !pluginsAvailable() ? "appearance" : requested);
      setPaneOpen(true);
    }
    setRevealedQuery("");
    clearSettingsTabRequest();
  });

  // Recording: capture the next chord for the command being remapped.
  const [recording, setRecording] = createSignal<string | null>(null);
  const [recordingIndex, setRecordingIndex] = createSignal<number | null>(0);
  const recordShortcut = (id: string, index: number | null = 0) => {
    const cancel = recording() === id && recordingIndex() === index;
    setRecordingIndex(index);
    setRecording(cancel ? null : id);
  };
  const removeShortcut = (id: string, index: number) => {
    setRecording(null);
    const row = shortcuts().find((row) => row.id === id)!;
    setShortcutOverride(id, shortcutValue(bindingAlternatives(row.effective).filter((_, i) => i !== index)));
  };
  const backToCategories = () => {
    setRecording(null);
    setSettingsQuery("");
    setRevealedQuery("");
    setPaneOpen(false);
  };
  // Settings owns its semantic Escape rungs.  Registering here (rather than in
  // App) keeps shortcut recording/search/disclosures from being skipped by a
  // blanket modal close and ensures disposal follows this component lifetime.
  createEffect(() => {
    if (!settingsOpen()) return;
    const unregister = registerTransientLayer({
      id: "settings",
      root: () => queryAllWindows<HTMLElement>(".settings-modal"),
      dismiss: (reason) => {
        if (recording()) { setRecording(null); return true; }
        if (settingsQuery()) { setSettingsQuery(""); return true; }
        if (reason === "back" && phone() && paneOpen()) { backToCategories(); return true; }
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
      const row = shortcuts().find((row) => row.id === id)!;
      const values = bindingAlternatives(row.effective);
      const index = recordingIndex();
      if (index === null) {
        // Adding keeps both the primary default and any built-in alias.
        if (row.alias && !row.overridden && !(id in (graphMeta()?.shortcuts ?? {}))) values.push(row.alias);
        values.push(b);
      } else values[index] = b;
      setShortcutOverride(id, shortcutValue([...new Set(values)]));
      setRecording(null);
    };
    onCleanup(listen(activeWindow(), "keydown", onKey, true));
  });

  return (
    <Show when={settingsOpen()}>
      <div class="modal-overlay" classList={{ "settings-maximized": settingsMaximized(), "settings-phone": phone() }} onClick={closeSettings}>
        <div ref={modalRoot} class="settings-modal" onClick={(e) => e.stopPropagation()}>
          <Show when={!phone() || (!paneOpen() && !searching())}>
          <aside class="settings-nav">
            <div class="settings-nav-title">{t("settings-ui-settings")}</div>
            <For each={availableTabs()}>
              {(t) => (
                <button
                  class="settings-nav-item"
                  data-settings-tab={t.id}
                  classList={{ active: (!phone() || paneOpen()) && tab() === t.id }}
                  onClick={() => { setSearchTarget(undefined); setRevealedQuery(""); setTab(t.id); setPaneOpen(true); }}
                >
                  {render(t.label)}
                </button>
              )}
            </For>
            <div class="settings-nav-foot">{t("settings-ui-built", { value1: buildStamp() })}</div>
          </aside>
          </Show>

          <div class="settings-pane">
            <div class="settings-pane-head">
              <Show when={phone() && (paneOpen() || searching())}><button type="button" class="icon-btn settings-back" aria-label={t("settings-back-to-list")} onClick={backToCategories}>‹</button></Show>
              <span class="settings-pane-title">{phone() && !paneOpen() ? t("settings-ui-settings") : render(availableTabs().find((t) => t.id === tab())!.label)}</span>
              <input
                ref={searchInput}
                class="settings-search-input"
                type="search"
                placeholder={shortcutSearch() ? t("settings-ui-search-shortcuts") : t("settings-ui-search-settings")}
                aria-label={shortcutSearch() ? t("settings-ui-search-shortcuts-2") : t("settings-ui-search-settings-2")}
                value={settingsQuery()}
                onInput={(event) => { setRevealedQuery(""); setSettingsQuery(event.currentTarget.value); }}
                onKeyDown={(event) => {
                  if (event.isComposing || event.keyCode === 229) return;
                  if (event.key === "Escape" && settingsQuery()) {
                    event.preventDefault();
                    setSettingsQuery("");
                    setRevealedQuery("");
                  }
                }}
              />
              <button class="icon-btn settings-maximize" type="button"
                aria-label={settingsMaximized() ? t("settings-ui-restore-settings-size") : t("settings-ui-maximize-settings")}
                aria-pressed={settingsMaximized()}
                onClick={() => setSettingsMaximized(!settingsMaximized())}>
                {settingsMaximized() ? "❐" : "□"}
              </button>
              <button class="icon-btn settings-close" aria-label={t("settings-close")} onClick={closeSettings}>
                ✕
              </button>
            </div>
            <Show when={!phone() || paneOpen() || searching()}><div class="settings-pane-body">
              <Show when={graphConfigUnreadable()}><p role="alert">{t("settings-ui-config-edn-could-not-be-read-this-graph-is")}</p></Show>
              <Suspense fallback={<div class="settings-pane-pending" aria-hidden="true" />}>
              <Show when={searching()}>
                <div class="settings-search-results" aria-live="polite">
                  <Show when={matches().length} fallback={<div class="settings-search-empty">{t("settings-ui-no-matching-settings")}</div>}>
                    <For each={matches()}>
                      {(entry) => (
                        <button type="button" class="settings-search-result" data-setting-target={entry.id} onClick={() => openSearchResult(entry)}>
                          <span><For each={settingHighlight(typeof entry.label === "string" ? entry.label : render(entry.label), settingsQuery())}>
                            {(part) => part.marked ? <mark>{part.text}</mark> : part.text}
                          </For><Show when={matchedDescription(entry, settingsQuery())}>{(description) => (
                            <span class="settings-search-description"><For each={settingHighlight(description(), settingsQuery())}>
                              {(part) => part.marked ? <mark>{part.text}</mark> : part.text}
                            </For></span>
                          )}</Show></span>
                          <small>{t("settings-search-result-location", { tab: render(TABS.find((candidate) => candidate.id === entry.tab)!.label), level: entry.level ?? "basic" })}</small>
                        </button>
                      )}
                    </For>
                  </Show>
                </div>
              </Show>
              <Show when={!searching()}>
              <Show when={tab() === "appearance"}>
                <AppearanceTab search={revealedQuery()} />
              </Show>
              <Show when={tab() === "themes"}><ThemeSettings search={revealedQuery()} /></Show>
              <Show when={tab() === "editor"}>
                <EditorTab search={revealedQuery()} />
              </Show>
              <Show when={tab() === "journals"}>
                <JournalsTab search={revealedQuery()} />
              </Show>
              <Show when={tab() === "files"}>
                <FilesTab search={revealedQuery()} />
              </Show>
              <Show when={tab() === "backups"}>
                <BackupsTab />
              </Show>
              <Show when={tab() === "graph"}>
                <GraphFolderSetting />
                <HomePageSetting />
                <GraphPublish /><QueryExportLimitSetting />
                <GitAutocommitSetting />
                <SeenSettings />
              </Show>
              <Show when={tab() === "sharing" && sharingAvailable()}>
                <QuickShareSettings />
              </Show>
              <Show when={tab() === "plugins" && pluginsAvailable()}>
                <div {...settingAttributes(settings["settings-ui-plugins"])}><PluginsTab target={searchTarget()} /></div>
              </Show>
              <Show when={tab() === "diagnostics"}>
                <DiagnosticsTab />
              </Show>
              <Show when={tab() === "shortcuts"}>
                <div {...settingAttributes(settings["settings-ui-keyboard-shortcuts"])}><ShortcutsSettingsPane
                  shortcuts={shortcuts()}
                  search={settingsQuery()}
                  recording={recording()}
                  recordingIndex={recordingIndex()}
                  onRecord={recordShortcut}
                  onRemove={removeShortcut}
                  onUnbind={(id) => { setRecording(null); setShortcutOverride(id, "false"); }}
                  onReset={(id) => { setRecording(null); resetShortcutOverride(id); }}
                /></div>
              </Show>
              <Show when={tab() === "about"}>
                <div {...settingAttributes(settings["settings-ui-about"])}><AboutTab /></div>
              </Show>
              </Show>
              </Suspense>
            </div></Show>
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
  setting: SettingDescriptor;
  hint?: JSX.Element;
  ogNote: Msg;
  ogValue: boolean;
  on: boolean;
  onToggle: () => void;
}): JSX.Element {
  const diverges = () => props.on !== props.ogValue;
  return (
    <div class="settings-field og-field" {...settingAttributes(props.setting)} classList={{ "og-diverges": diverges() }}>
      <div class="settings-field-row">
        <span class="settings-label">
          {settingLabel(props.setting)}
          <Show when={diverges()}>
            <span class="og-badge" title={t("settings-ui-tine-s-default-differs-from-logseq-here")}>{t("settings-ui-differs-from-logseq")}</span>
          </Show>
        </span>
        <div class="settings-field-control">
          <Toggle on={props.on} onClick={props.onToggle} />
        </div>
      </div>
      <Show when={props.hint ?? props.setting.description}>
        <div class="settings-hint settings-field-hint">{props.hint ?? settingDescription(props.setting)}</div>
      </Show>
      <div class="og-note">
        <span class="og-logseq">{render(props.ogNote)}</span>
        <Show when={diverges()}>
          <button class="og-revert" onClick={props.onToggle}>{t("settings-ui-match-logseq")}</button>
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
  catch { pushToast(msg("settings-ui-could-not-load-advanced-section-preference"), "error"); }
  const [open, setOpen] = createSignal(initial);
  let button: HTMLButtonElement | undefined;
  const expanded = () => props.forceOpen || open();
  const persist = (value: boolean) => {
    try { localStorage.setItem(key, value ? "1" : "0"); }
    catch { pushToast(msg("settings-ui-could-not-save-advanced-section-preference"), "error"); }
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
      ><Rich message={msg("settings-ui-advanced", { choice1: expanded() ? "yes" : "no" })} slots={{ "slot1": (children) => <span aria-hidden="true">{children}</span> }} /></button>
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
      pushToast(msg("settings-ui-saved-restart-tine-to-apply-the-window-frame-change"), "info");
    } catch (error) {
      pushToast(msg("settings-ui-couldn-t-save-the-window-frame-setting", { value1: String(error) }), "error");
    } finally {
      setSavingNativeFrame(false);
    }
  };

  return (
    <>
      <LanguageSetting />
      <ToolbarSettings />

      <div class="settings-row" {...settingAttributes(settings["settings-ui-mode"])}>
        <span class="settings-label">{settingLabel(settings["settings-ui-mode"])}</span>
        <ThemeChoice />
      </div>

      <div class="settings-row" {...settingAttributes(settings["settings-ui-accent-color"])}>
        <span class="settings-label">{settingLabel(settings["settings-ui-accent-color"])}</span>
        <div>
          <input
            type="color"
            class="settings-color"
            value={accentColor() ?? "#2563eb"}
            onInput={(e) => changeAccent(e.currentTarget.value)}
          />
          <Show when={accentColor()}>
            <button class="settings-btn" style={{ "margin-left": "8px" }} onClick={() => changeAccent(null)}>{t("settings-ui-reset")}</button>
          </Show>
        </div>
      </div>

      <Field
        setting={settings["settings-ui-interface-size"]}

      >
        <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
          <button class="settings-btn" title={t("settings-ui-smaller-ctrl")} onClick={zoomOut}>−</button>
          <span class="mono" style={{ "min-width": "3.4em", "text-align": "center" }}>
            {Math.round(interfaceZoom() * 100)}%
          </span>
          <button class="settings-btn" title={t("settings-ui-larger-ctrl")} onClick={zoomIn}>+</button>
          <Show when={interfaceZoom() !== 1}>
            <button class="settings-btn" style={{ "margin-left": "8px" }} onClick={zoomReset}>{t("settings-ui-reset")}</button>
          </Show>
        </div>
      </Field>

      {/* GH #716: a touch-first device (phone/tablet) keeps Sidebar, Search, Back and Forward in
          the top bar; this opt-in restores the GH #598 bottom row. */}
      <Show when={touchPrimary()}>
        <Field setting={settings["settings-mobile-navigation-label"]}>
          <Toggle on={mobileNavigationAtBottom()} onClick={() => setMobileNavigationAtBottom(!mobileNavigationAtBottom())} />
        </Field>
      </Show>

      <Field setting={settings["settings-ui-wide-mode"]}>
        <Toggle on={wideMode()} onClick={toggleWideMode} />
      </Field>
      <Field setting={settings["settings-ui-document-mode"]}>
        <Toggle on={documentMode()} onClick={toggleDocumentMode} />
      </Field>
      <Field
        setting={settings["settings-ui-document-mode-enter-creates-a-new-block"]}
        hint={<><Rich message={settingHint(settings["settings-ui-document-mode-enter-creates-a-new-block"])} /></>}
      >
        <Toggle
          on={docModeEnterForNewBlock()}
          onClick={() => changeDocModeEnterForNewBlock(!docModeEnterForNewBlock())}
        />
      </Field>
      <AnnotationVisibilitySettings />
      <Field
        setting={settings["settings-ui-show-brackets"]}
        hint={<><Rich message={settingHint(settings["settings-ui-show-brackets"])} /></>}
      >
        <Toggle on={showBrackets()} onClick={() => changeShowBrackets(!showBrackets())} />
      </Field>
      <Field
        setting={settings["settings-ui-typographic-replacements"]}

      >
        <select
          class="settings-select"
          value={typographyMode()}
          onChange={(e) => {
            const v = e.currentTarget.value;
            setTypographyMode(v === "off" ? "off" : v === "type" ? "type" : "render");
          }}
        >
          <option value="render">{t("settings-ui-on-while-reading")}</option>
          <option value="type">{t("settings-ui-on-while-typing")}</option>
          <option value="off">{t("settings-ui-off")}</option>
        </select>
      </Field>

      <Field setting={settings["settings-ui-wrap-code-lines"]}>
        <Toggle on={codeWrapping()} onClick={() => changeCodeWrapping(!codeWrapping())} />
      </Field>
      <Field setting={settings["setting-sticky-parents"]}>
        <Toggle on={stickyParents()} onClick={() => changeStickyParents(!stickyParents())} />
      </Field>
      <Field
        setting={settings["settings-ui-auto-pair-brackets-quotes"]}

      >
        <Toggle on={autoPairing()} onClick={() => setAutoPairing(!autoPairing())} />
      </Field>

      <OgField
        setting={settings["settings-ui-space-after-inserting-a-reference"]}

        ogNote={msg("settings-og-reference-spacing")}
        ogValue={false}
        on={spaceAfterRefCompletion()}
        onToggle={() => setSpaceAfterRefCompletion(!spaceAfterRefCompletion())}
      />

      <Field
        setting={settings["settings-ui-dim-in-focus-mode"]}

      >
        <Toggle on={dimInFocus()} onClick={() => setDimInFocus(!dimInFocus())} />
      </Field>

      <Field
        setting={settings["settings-ui-load-local-file-images"]}

      >
        <Toggle on={allowLocalFileImages()} onClick={() => setAllowLocalFileImages(!allowLocalFileImages())} />
      </Field>

      <AdvancedSection tab="appearance" forceOpen={advancedMatch("appearance", props.search)}>
      <Field setting={settings["settings-show-workspaces"]}>
        <Toggle on={showWorkspaces()} onClick={() => setShowWorkspaces(!showWorkspaces())} />
      </Field>
      <Field setting={settings["settings-show-help-button"]}>
        <Toggle on={showHelpButton()} onClick={() => setShowHelpButton(!showHelpButton())} />
      </Field>
      <Field
        setting={settings["settings-ui-right-to-left-note-layout"]}

      >
        <Toggle on={noteLayoutRtl()} onClick={() => setNoteLayoutRtl(!noteLayoutRtl())} />
      </Field>
        <StyleKnobsSettings />
        <CustomCssSettings />
        <ContentWidthFields />
        <Field
          setting={settings["settings-ui-smooth-scrolling-experimental"]}

        >
          <Toggle on={smoothScrollEnabled()} onClick={() => setSmoothScroll(!smoothScrollEnabled())} />
        </Field>
      </AdvancedSection>

      {/* Window chrome. macOS always uses its native frame (rounded corners +
          traffic lights, via the build-time Overlay title bar), so the toggle is
          only meaningful — and only shown — on Linux/Windows. */}
      <Show when={isTauri() && !isMac}>
        <Field
          setting={settings["settings-ui-system-title-bar-window-controls"]}

        >
          <Toggle on={nativeFrameEnabled()} onClick={() => void changeNativeFrame()} />
        </Field>
      </Show>
      <Show when={isTauri() && isMac}>
        <Field
          setting={settings["settings-ui-window-controls"]}

        >
          <span style={{ color: "var(--text-muted)", "font-size": "12px" }}>{t("settings-ui-native-macos")}</span>
        </Field>
      </Show>

      {/* System tray (GH #625): renders nothing unless the native side has one. */}
      <TraySettings />

    </>
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
      <CommentNameSetting />
      <LinkSchemeSettings />
      <Field setting={settings["settings-auto-embed-video-links"]}>
        <Toggle on={autoEmbedVideoLinks()} onClick={() => setAutoEmbedVideoLinks(!autoEmbedVideoLinks())} />
      </Field>
      <Field
        setting={settings["settings-ui-file-format"]}
        hint={
          <><Rich message={settingHint(settings["settings-ui-file-format"])} /></>
        }
      >
        <div class="settings-segment">
          <button
            classList={{ active: (graphMeta()?.preferred_format ?? "md") === "md" }}
            onClick={() => changePreferredFormat("md")}
          >{t("settings-ui-markdown")}</button>
          <button
            classList={{ active: graphMeta()?.preferred_format === "org" }}
            onClick={() => changePreferredFormat("org")}
          >{t("settings-ui-org")}</button>
        </div>
      </Field>

      <Field
        setting={settings["settings-ui-spell-checker"]}
        hint={
          <><Rich message={settingHint(settings["settings-ui-spell-checker"])} /></>
        }
      >
        <Toggle on={spellcheckEnabled()} onClick={() => setSpellcheckEnabled(!spellcheckEnabled())} />
      </Field>

      <Field
        setting={settings["settings-ui-logical-outdenting"]}
        hint={<><Rich message={settingHint(settings["settings-ui-logical-outdenting"])} /></>}
      >
        <Toggle on={logicalOutdenting()} onClick={() => changeLogicalOutdenting(!logicalOutdenting())} />
      </Field>

      <Show when={spellcheckEnabled()}>
        <Field
          setting={settings["settings-ui-spellcheck-languages"]}
          hint={
            <><Rich message={settingHint(settings["settings-ui-spellcheck-languages"])} /></>
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
                    <span class="spellcheck-dict-missing">{t("settings-ui-not-installed")}</span>
                  </Show>
                </label>
              )}
            </For>
            <Show when={dictRows().length === 0}>
              <div class="spellcheck-empty"><Rich message={msg("settings-ui-no-dictionaries-found-install-one-e-g-hunspell-en")} /></div>
            </Show>
            <button class="spellcheck-rescan" type="button" onClick={() => void loadDictionaries()}>{t("settings-ui-rescan")}</button>
          </div>
        </Field>
      </Show>

      <AdvancedSection tab="editor" forceOpen={advancedMatch("editor", props.search)}>
        <Field setting={settings["settings-reference-previews"]}>
          <Toggle on={previewsEnabled()} onClick={() => changePreviewsEnabled(!previewsEnabled())} />
        </Field>
        <Field setting={settings["settings-hover-preview-delay"]}>
          <input
            type="number" min="0" step="any"
            aria-label={t("settings-hover-preview-delay-seconds")}
            value={hoverPreviewDelaySeconds()}
            onChange={event => {
              const input = event.currentTarget;
              if (input.value.trim() !== "") setHoverPreviewDelaySeconds(Number(input.value));
              input.value = String(hoverPreviewDelaySeconds());
            }}
          />
        </Field>
        <Field setting={settings["settings-cm6-label"]}>
          <Toggle on={!cm6EditorEnabled()} onClick={() => setCm6EditorEnabled(!cm6EditorEnabled())} />
        </Field>
        <Field
          setting={settings["settings-ui-link-autocomplete-default"]}

        >
          <select
            aria-label={t("settings-ui-link-autocomplete-default")}
            value={linkAutocompletePolicy()}
            onChange={(event) => setLinkAutocompletePolicy(event.currentTarget.value as LinkAutocompletePolicy)}
          >
            <option value="adaptive">{t("settings-ui-og-adaptive")}</option>
            <option value="existing">{t("settings-ui-prefer-existing")}</option>
            <option value="typed">{t("settings-ui-prefer-exactly-what-i-typed")}</option>
          </select>
        </Field>

        <Field
          setting={settings["settings-ui-switch-to-an-already-open-tab-when-navigating"]}

        >
          <Toggle on={navReuseTabs()} onClick={() => setNavReuseTabs(!navReuseTabs())} />
        </Field>

        <Field setting={settings["settings-result-destination-label"]}>
          <select
            aria-label={render(msg("settings-result-destination-label"))}
            value={searchResultsOpenInThisTab() ? "here" : "tab"}
            onChange={(event) => setSearchResultsOpenInThisTab(event.currentTarget.value === "here")}
          >
            <option value="tab">{render(msg("settings-result-destination-new-tab"))}</option>
            <option value="here">{render(msg("settings-result-destination-this-tab"))}</option>
          </select>
        </Field>

        <Field setting={settings["settings-pinyin-label"]}>
          <Toggle on={pinyinMatchingEnabled()} onClick={() => setPinyinMatchingEnabled(!pinyinMatchingEnabled())} />
          <button type="button" class="og-revert" onClick={resetPinyinMatchingSetting}>
            {render(msg("settings-pinyin-reset"))}
          </button>
        </Field>

        <Field
          setting={settings["settings-ui-learn-ctrl-k-choices"]}

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
                pushToast(msg("settings-ui-ctrl-k-ranking-reset-for-this-graph"));
              }}
            >{t("settings-ui-reset-ranking")}</button>
          </>
        </Field>

        <OgField
        setting={settings["settings-ui-strip-collapsed-when-copying"]}

        ogNote={msg("settings-og-copy-collapsed")}
        ogValue={false}
        on={copyStripCollapsed()}
        onToggle={() => setCopyStripCollapsed(!copyStripCollapsed())}
        />
      </AdvancedSection>

      <OgField
        setting={settings["settings-ui-click-a-block-reference-to-zoom-in"]}

        ogNote={msg("settings-og-reference-zoom")}
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
      (next) => backend().setCaptureEnterFiles(next), msg("toast-save-capture-enter-failed"));
  };
  return (
    <>
      <Field
        setting={settings["settings-ui-journal-date-format"]}
        hint={
          <><Rich message={settingHint(settings["settings-ui-journal-date-format"])} /></>
        }
      >
        <DateFormatSelect />
        <For each={graphMeta()?.journal_config_diagnostics ?? []}>
          {(message) => <div class="config-diagnostic" role="status">{t("journal-config-diagnostic", { message })}</div>}
        </For>
      </Field>

      <Field
        setting={settings["settings-ui-first-day-of-week"]}
        hint={
          <><Rich message={settingHint(settings["settings-ui-first-day-of-week"])} /></>
        }
      >
        <select
          class="settings-select"
          value={String(graphMeta()?.start_of_week ?? 6)}
          onChange={(e) => changeStartOfWeek(Number(e.currentTarget.value))}
        >
          {/* Logseq convention: 0=Monday … 6=Sunday. */}
          <option value="0">{t("settings-ui-monday")}</option>
          <option value="1">{t("settings-ui-tuesday")}</option>
          <option value="2">{t("settings-ui-wednesday")}</option>
          <option value="3">{t("settings-ui-thursday")}</option>
          <option value="4">{t("settings-ui-friday")}</option>
          <option value="5">{t("settings-ui-saturday")}</option>
          <option value="6">{t("settings-ui-sunday")}</option>
        </select>
      </Field>

      <Field
        setting={settings["settings-ui-show-carry-over-buttons"]}

      >
        <Toggle on={showCarryButtons()} onClick={() => setShowCarryButtons(!showCarryButtons())} />
      </Field>

      <Field
        setting={settings["settings-ui-carry-over-keeps-context"]}

      >
        <Toggle on={carryKeepsContext()} onClick={() => setCarryKeepsContext(!carryKeepsContext())} />
      </Field>

      <Field setting={settings["settings-ui-carry-over-header"]}>
        <Toggle on={carryHeader()} onClick={() => setCarryHeader(!carryHeader())} />
      </Field>

      <Field
        setting={settings["settings-ui-carry-last-n-days"]}

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
        setting={settings["settings-ui-task-workflow"]}
        hint={
          <><Rich message={settingHint(settings["settings-ui-task-workflow"])} /></>
        }
      >
        <div class="settings-segment">
          <button
            classList={{ active: workflow() === "todo" }}
            onClick={() => changeWorkflow("todo")}
          >{t("settings-ui-todo-doing")}</button>
          <button classList={{ active: workflow() === "now" }} onClick={() => changeWorkflow("now")}>{t("settings-ui-now-later")}</button>
        </div>
      </Field>

      <Field
        setting={settings["settings-ui-time-tracking"]}
        hint={
          <><Rich message={settingHint(settings["settings-ui-time-tracking"], { choice1: graphMeta()?.logbook_with_second_support ?? true ? "yes" : "no" })} /></>
        }
      >
        <Toggle on={timetrackingEnabled()} onClick={() => changeTimetrackingEnabled(!timetrackingEnabled())} />
      </Field>

      <JournalTemplateField />

      <AdvancedSection tab="journals" forceOpen={advancedMatch("journals", props.search)}>
        <Field
          setting={settings["settings-ui-quick-capture-enter-key"]}

        >
          <Toggle on={captureEnterFiles()} onClick={toggleCaptureEnter} />
        </Field>
      </AdvancedSection>

      <Field
        setting={settings["settings-ui-agenda-window"]}
        hint={
          <>
            {settingDescription(settings["settings-ui-agenda-window"])}
            <SectionsPageNote />
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
        <span class="settings-hint">{t("settings-ui-days-back")}</span>
        <input
          type="number"
          min="0"
          max="3650"
          class="settings-num"
          value={graphMeta()?.scheduled_future_days ?? agendaDaysAhead()}
          disabled={graphMeta()?.scheduled_future_days != null}
          onChange={(e) => setAgendaDaysAhead(Number(e.currentTarget.value))}
        />
        <span class="settings-hint">{t("settings-agenda-days-ahead", { source: graphMeta()?.scheduled_future_days != null ? "graph" : "device" })}</span>
      </Field>
    </>
  );
}

/** GH #605: with a sections page, today's sections (agenda included) are the
 *  queries written on it, so this window no longer applies. Say where they live.
 *  It goes in the setting's full-width hint, never its control column: that column
 *  does not shrink, so a sentence there ran off the pane and clipped "Open page" (GH #808). */
function SectionsPageNote(): JSX.Element {
  // Keep the optional note empty while loading: an undefined first value must
  // neither crash its index lookup nor suspend the entire Settings dialog.
  const [pages] = createResource(() => changedAny(markerChangeKeys(JOURNAL_SECTIONS_MARKER)), () => locateSectionsPages(), { initialValue: [] as string[] });
  const page = () => readOr(pages, [] as string[], "sections page note")[0];
  return (
    <Show when={page()}>
      {(name) => <div class="settings-sections-note">
        {render(msg("settings-sections-page", { page: name() }))}{" "}
        <button type="button" class="settings-link" onClick={() => { closeSettings(); openPage(name()); }}>{render(msg("settings-sections-open"))}</button>
      </div>}
    </Show>
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
      pushToast(msg("settings-ui-couldn-t-save", { value1: String(e) }), "error");
    }
  };

  const restore = (b: BackupInfo) => {
    if (!ready() || busy() || b.status !== "ok") return;
    void restoreBackupFromSettings(b, fmtStamp(b.stamp), setBusy, () => { void refresh(); });
  };

  const deleteOld = async (b: BackupInfo) => {
    const remove = backend().deleteOldBackups;
    if (!remove || !b.old_root || !ready() || busy()) return;
    const owner = bindingOwner(() => alive);
    setBusy(true);
    try {
      const confirmed = await readOwned(owner,backend().confirm(t("backup-delete-old-confirm", { path: b.old_root }),
        { okLabel:t("backup-delete-old"),cancelLabel:t("confirm-cancel") }));
      if (confirmed.kind === "stale" || !confirmed.value) return;
      const result = await writeOwned(owner,remove.call(backend(),b.id));
      if (result.kind !== "stale") void refresh();
    } catch (error) {
      pushToast(msg("backup-delete-old-failed", { error:String(error) }),"error");
    } finally { if (owner()) setBusy(false); }
  };

  return (
    <>
      <div class="settings-hint settings-block">{settingDescription(settings["settings-available-snapshots"])}</div>

      <Field setting={settings["settings-ui-snapshots-to-keep"]}>
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

      <div class="settings-section" {...settingAttributes(settings["settings-available-snapshots"])}>
        {settingLabel(settings["settings-available-snapshots"])}
        <button
          class="settings-btn"
          style={{ "margin-left": "10px" }}
          disabled={loading() || busy()}
          onClick={() => void refresh()}
        >{t("settings-ui-refresh")}</button>
      </div>
      <Show when={loading()}>
        <div class="settings-hint settings-block" role="status">{t("settings-ui-loading-snapshot-settings")}</div>
      </Show>
      <Show when={loadError()}>
        {(error) => (
          <div class="settings-hint settings-block" role="alert">
            {t("settings-backup-load-failed", { error: error() })}
            <button class="settings-btn" style={{ "margin-left": "10px" }} onClick={() => void refresh()}>{t("settings-ui-retry")}</button>
          </div>
        )}
      </Show>
      <Show
        when={ready() && list().length}
        fallback={
          <Show when={ready()}>
            <div class="settings-hint settings-block">{t("settings-ui-no-snapshots-yet")}</div>
          </Show>
        }
      >
        <div class="settings-backups">
          <For each={list()}>
            {(b) => (
              <div class="settings-backup-row">
                <span class="settings-backup-when">{fmtStamp(b.stamp)}
                  <Show when={b.old_root}>{(path) => <span class="settings-backup-origin">{t("backup-old-location",{ path:path() })}</span>}</Show>
                </span>
                <span class="settings-backup-files mono">
                  {b.status === "ok"
                    ? t("settings-ui-files-2", { files: b.files })
                    : t("settings-ui-files-backup-status", { files: b.files, status: t(BACKUP_STATUS_KEY[b.status]) })}
                </span>
                <div class="settings-backup-actions">
                  <button class="settings-btn" disabled={!ready() || busy() || b.status !== "ok"} onClick={() => void restore(b)}>{t("settings-ui-restore")}</button>
                  <Show when={b.old_root && backend().deleteOldBackups}><button class="settings-btn" disabled={!ready() || busy()} onClick={() => void deleteOld(b)}>{t("backup-delete-old")}</button></Show>
                </div>
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
  const reconcile = async (op: () => Promise<void>, ok: Msg) => {
    const owner = bindingOwner();
    try {
      const result = await writeOwned(owner, op());
      if (result.kind === "stale") return;
      pushToast(ok, "success");
      await refreshJournalConflicts();
    } catch (e) {
      pushToast(msg("settings-ui-couldn-t-do-that", { value1: String(e) }), "error");
    }
  };
  const trashFile = async (name: string) => {
    const owner = bindingOwner();
    const confirmed = await readOwned(owner, backend().confirm(
        t("settings-ui-move-the-journal-file-to-the-trash-it-s", { name: name }),
        { okLabel: t("confirm-move-to-trash"), cancelLabel: t("confirm-cancel") },
      ));
    if (confirmed.kind === "stale" || !confirmed.value) return;
    await writeOwned(owner, reconcile(() => backend().trashJournalFile(name, "delete-page"), msg("settings-ui-moved-to-trash", { name })));
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
      <div class="settings-section" style={{ "margin-top": "18px" }} {...settingAttributes(settings["settings-ui-duplicate-journal-days"])}>{settingLabel(settings["settings-ui-duplicate-journal-days"])}</div>
      <div class="settings-hint settings-block"><Rich message={settingHint(settings["settings-ui-duplicate-journal-days"])} /></div>
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
                      msg("settings-journal-file-merged", { name: f.name, canonical: canonical.name }),
                    ) : undefined}
                    onRename={(n) => void reconcile(
                      () => backend().renameFileToPage(f.path, n, "rename-page"),
                      msg("settings-journal-file-renamed", { name: f.name, next: n })
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
      <div class="settings-section" style={{ "margin-top": "18px" }} {...settingAttributes(settings["settings-ui-conflicts"])}>{settingLabel(settings["settings-ui-conflicts"])}</div>
      <div class="settings-hint settings-block"><Rich message={msg("settings-conflicts-summary", { count: pendingConflictCount() })} /></div>
      <span class="journal-conflict-actions">
        <button class="settings-btn" onClick={() => { openConflicts(); closeSettings(); }}>{settingDescription(settings["settings-ui-conflicts"])}</button>
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
      (next) => backend().setWatchMode(next), msg("toast-save-file-watcher-failed"));
  };
  return (
    <>
      <div class="settings-section">{t("settings-ui-asset-names")}</div>
      <Field
        setting={settings["settings-ui-new-asset-filename"]}
        hint={
          <><Rich message={settingHint(settings["settings-ui-new-asset-filename"])} /></>
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
          >{t("settings-ui-original-name")}</button>
          <button
            classList={{ active: assetNameFormat() === STAMPED_ASSET_NAME_FORMAT }}
            onClick={() => setAssetNameFormat(STAMPED_ASSET_NAME_FORMAT)}
          >{t("settings-ui-date-name")}</button>
        </div>
        <div class="asset-name-preview">
          <div><Rich message={msg("settings-ui-drag-holiday-photo-jpg", { value1: dragExample() })} slots={{ "slot1": (children) => <code class="mono">{children}</code>, "slot2": (children) => <code class="mono">{children}</code> }} /></div>
          <div><Rich message={msg("settings-ui-paste-an-image", { value1: pasteExample() })} slots={{ "slot1": (children) => <code class="mono">{children}</code> }} /></div>
        </div>
      </div>

      <Field
        setting={settings["settings-ui-watch-for-external-edits"]}
        hint={
          <><Rich message={settingHint(settings["settings-ui-watch-for-external-edits"])} slots={{ "slot1": (children) => <b>{children}</b>, "slot2": (children) => <b>{children}</b> }} /></>
        }
      >
        <div class="settings-segment">
          <button
            classList={{ active: watchMode() === "inotify" }}
            onClick={() => changeWatchMode("inotify")}
          >{t("settings-ui-live-inotify")}</button>
          <button
            classList={{ active: watchMode() === "poll" }}
            onClick={() => changeWatchMode("poll")}
          >{t("settings-ui-poll-3s")}</button>
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
        pushToast(msg("settings-ui-found", { command: command }), "success");
      } else {
        pushToast(msg("settings-ui-couldn-t-find-it-set-the-command-manually"), "error");
      }
    } catch {
      pushToast(msg("settings-ui-autodetect-failed"), "error");
    } finally {
      setDetecting(null);
    }
  };
  return (
    <>
      <div class="settings-section" {...settingAttributes(settings["settings-ui-diagram-editors"])}>{settingLabel(settings["settings-ui-diagram-editors"])}</div>
      <div class="settings-hint" style={{ "margin-bottom": "8px" }}><Rich message={settingHint(settings["settings-ui-diagram-editors"])} slots={{ "slot1": (children) => <code class="mono">{children}</code>, "slot2": (children) => <code class="mono">{children}</code> }} /></div>
      <For each={MEDIA_EDITORS}>
        {(ed) => (
          <Field setting={mediaEditorSetting(ed)}>
            <div class="media-editor-row">
              <input
                type="text"
                class="settings-input mono"
                placeholder={t("settings-ui-system-default-opener")}
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
                  {detecting() === ed.id ? t("settings-detecting") : t("settings-autodetect")}
                </button>
              </Show>
            </div>
          </Field>
        )}
      </For>
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

/** Why a listed backup cannot be restored (the native restore refuses it too). */
const BACKUP_STATUS_KEY = {
  damaged: "settings-ui-backup-damaged",
  unsupported: "settings-ui-backup-unsupported",
} as const satisfies Record<Exclude<BackupInfo["status"], "ok">, string>;

// "2026-06-17_14-30-05" (UTC) → a readable local timestamp.
function fmtStamp(s: string): string {
  const m = s.match(/^(\d{4})-(\d{2})-(\d{2})_(\d{2})-(\d{2})-(\d{2})$/);
  if (!m) return s;
  const d = new Date(`${m[1]}-${m[2]}-${m[3]}T${m[4]}:${m[5]}:${m[6]}Z`);
  return isNaN(d.getTime()) ? s : d.toLocaleString();
}

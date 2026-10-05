import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { reportUiFailure } from "./uiFailure";
import { graphMeta, setGraphMeta, bumpGraphEpoch } from "./graphSession";
import { pushToast } from "./toasts";
import { isMobilePlatform } from "./nativeChrome";
// Small global UI state: theme, left sidebar, and the quick-switcher modal.
import { createEffect, createRoot, createSignal, useContext } from "solid-js";
import type { JournalConflict, PageKind } from "./types";
import type { OwnedPluginBlockSnapshot } from "./plugins/ownership";
import { backend } from "./backend";
import { setFocusFullscreen } from "./focusFullscreen";
import { captureBinding, clearOnBindingInvalidated, graphScopedSignal } from "./binding";
import { graphOwner, latestOwner, ownedWhen, readOwned, readOwnedResource, writeOwned } from "./owned";
// Zoom is route state; these are call-time only, so the ui↔router cycle is safe.
import { route, focusBlock, scheduleSessionSave, openPageTarget } from "./routerBridge";
import { beginConflictRefresh, conflictQueue, conflictRefreshCurrent, forgetArrivalNotice, setConflictInventory, trackArrivalNotice } from "./conflictQueue";
export { conflictQueue, settleArtifactConflict, syncConflicts, setSyncConflicts } from "./conflictQueue";
import { parseBlockPos, type PageTarget } from "./routeTypes";
import { PaneContext } from "./paneContext";
import { exitPaneSelect } from "./paneSelect";
import { setJournalTitleFormat } from "./journal";
import { pageIdentityKey } from "./pageIdentity";
import { clearDrawerOpener, mobileDrawerMode, captureDrawerOpener, restoreDrawerFocus, type DrawerSide } from "./mobileDrawers";
import { navigationName } from "./pageIndex";
import { forgetDeletedFavorite, renameFavorite } from "./favorites";
import { changeGraphSetting, writeGraphSignal } from "./graphPreferences";

export { appearancePreference, theme, resolveTheme, applyTheme, setAppearancePreference } from "./themePreference";
export type { ThemePreference } from "./themePreference";
import { theme, setAppearancePreference } from "./themePreference";

// Task workflow from config.edn (:preferred-workflow): drives mod+enter cycling.
export const [workflow, setWorkflow] = createSignal<"now" | "todo">("now");
/** Set workflow optimistically and persist to graph config; failure rolls back
 * and toasts. The file is re-read on next graph open. */
export function changeWorkflow(wf: "now" | "todo") {
  if (wf === workflow()) return;
  writeGraphSignal("workflow", workflow, setWorkflow, wf,
    (next) => backend().setPreferredWorkflow(next), "preferred workflow");
}

export function timetrackingEnabled(): boolean {
  return graphMeta()?.enable_timetracking ?? true;
}

export function logbookWithSecondSupport(): boolean {
  return graphMeta()?.logbook_with_second_support ?? true;
}

export function changeTimetrackingEnabled(enabled: boolean) {
  changeGraphSetting("enable_timetracking", enabled, (next) => backend().setTimetrackingEnabled(next!), "time tracking preference");
}

export function showBrackets(): boolean {
  return graphMeta()?.show_brackets ?? true;
}

export function changeShowBrackets(on: boolean) {
  changeGraphSetting("show_brackets", on, (next) => backend().setShowBrackets(next), "bracket display preference");
}

/** In document mode, should plain Enter retain the ordinary structural split?
 *  The default false follows OG's `:shortcut/doc-mode-enter-for-new-block?`
 *  switch (`src/main/frontend/state.cljs:714-717` at `6e7afa8eb`). */
export function docModeEnterForNewBlock(): boolean {
  return graphMeta()?.doc_mode_enter_for_new_block ?? false;
}

export function changeDocModeEnterForNewBlock(on: boolean) {
  changeGraphSetting("doc_mode_enter_for_new_block", on, (next) => backend().setDocModeEnterForNewBlock(next!), "document mode Enter preference");
}

/** Logical (Roam-like) outdenting leaves following siblings under their current
 *  parent. OG uses `:editor/logical-outdenting?` for this (`src/main/frontend/modules/outliner/core.cljs:835-852`
 *  at `6e7afa8eb`). */
export function logicalOutdenting(): boolean {
  return graphMeta()?.logical_outdenting ?? false;
}

export function changeLogicalOutdenting(on: boolean) {
  changeGraphSetting("logical_outdenting", on, (next) => backend().setLogicalOutdenting(next!), "logical outdenting preference");
}

// --- appearance: accent color, wide mode, document mode (all persisted) ---
function loadStr(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    if (typeof localStorage !== "undefined") pushToast("Could not load display preference.", "error");
    return null;
  }
}
function saveStr(key: string, val: string | null, what = "display preference"): boolean {
  try {
    if (val === null) localStorage.removeItem(key);
    else localStorage.setItem(key, val);
    return true;
  } catch {
    pushToast(`Could not save ${what}.`, "error");
    return false;
  }
}

/** Persist the device theme through the shared display-preference writer.
 * Returns whether storage accepted it; errors show the existing toast. */
export function persistThemePreference(key: string, value: string): boolean {
  return saveStr(key, value);
}

const ACCENT_KEY = "logseq-claude.accent";
export const [accentColor, setAccentColor] = createSignal<string | null>(loadStr(ACCENT_KEY));
/** Apply (or clear) the accent color as the link/highlight CSS variables. */
export function applyAccent() {
  const c = accentColor();
  const root = document.documentElement;
  if (c) {
    root.style.setProperty("--accent", c);
    root.style.setProperty("--link-color", c);
  } else {
    root.style.removeProperty("--accent");
    root.style.removeProperty("--link-color");
  }
}
export function changeAccent(c: string | null) {
  if (!saveStr(ACCENT_KEY, c)) return;
  setAccentColor(c);
  applyAccent();
}

const WIDE_KEY = "logseq-claude.wide";
const DOC_KEY = "logseq-claude.doc-mode";
export const [wideMode, setWideMode] = createSignal(loadStr(WIDE_KEY) === "1");
export const [documentMode, setDocumentMode] = createSignal(loadStr(DOC_KEY) === "1");
export function toggleWideMode() {
  const v = !wideMode();
  if (!saveStr(WIDE_KEY, v ? "1" : null)) return;
  setWideMode(v);
}
export function toggleDocumentMode() {
  const v = !documentMode();
  if (!saveStr(DOC_KEY, v ? "1" : null)) return;
  setDocumentMode(v);
}

// --- typographic replacements (a "Differs from Logseq" opinion): render `->` as
// `→`, `--` as `–`, `---` as `—`, etc.
//   "render" = source keeps the ASCII, only the rendered view shows glyphs
//              (default; mirrors how \Delta→Δ works).
//   "type"   = rewrite the SOURCE as you type (the .md itself gets the glyphs);
//              the render pass is then off since the source already holds them.
//   "off"    = leave the ASCII everywhere.
// Local appearance pref, like wide/document mode. ---
export type TypographyMode = "off" | "render" | "type";
const TYPO_KEY = "logseq-claude.typography";
function loadTypographyMode(): TypographyMode {
  const v = loadStr(TYPO_KEY);
  return v === "off" ? "off" : v === "type" ? "type" : "render";
}
export const [typographyMode, setTypographyModeSig] =
  createSignal<TypographyMode>(loadTypographyMode());
export function setTypographyMode(m: TypographyMode) {
  if (!saveStr(TYPO_KEY, m === "render" ? null : m)) return;
  setTypographyModeSig(m);
  bumpGraphEpoch(); // re-render open pages so the change is immediate
}

// --- editor auto-pairing (ON by default, OG parity, GH #291): typing `(`/`[`/
// `{`/`"`/backtick inserts the matching closer (caret between), wraps a selection,
// types-through a closer, and Backspace deletes an empty pair. The always-on
// OG-style `[[`→`[[]]` page-ref pairing is separate (autoPairEdit) and unaffected.
// Local editor pref: only an explicit opt-out is stored ("0"). ---
const AUTOPAIR_KEY = "logseq-claude.autopair";
export const [autoPairing, setAutoPairingSig] = createSignal(loadStr(AUTOPAIR_KEY) !== "0");
export function setAutoPairing(v: boolean) {
  if (!saveStr(AUTOPAIR_KEY, v ? null : "0")) return;
  setAutoPairingSig(v);
}

// --- first day of week (calendar + scheduled/deadline date pickers) ---
// Sourced from config.edn `:start-of-week` (Logseq's convention: 0=Monday …
// 6=Sunday, default 6=Sunday), so it round-trips with Logseq. The earlier
// Saturday-first bug was feeding that Monday-based index straight into JS
// getDay() (Sunday-based); convert with (L+1)%7. Changing it (Settings → all 7
// days) writes config.edn.
const LOGSEQ_TO_JS_DOW = (l: number): number => ((l >= 0 && l <= 6 ? l : 6) + 1) % 7;
/** First day of week as a JS getDay() index (0=Sunday … 6=Saturday). Reactive on
 *  graphMeta so the pickers re-render when it changes. */
export function firstDayOfWeek(): number {
  return LOGSEQ_TO_JS_DOW(graphMeta()?.start_of_week ?? 6);
}
/** Persist a new first-day-of-week (Logseq index 0=Monday … 6=Sunday) to
 *  config.edn and update graphMeta optimistically so the calendar reflects it
 *  immediately; invalid indexes clamp to 0..6 and failed writes roll back with a toast. */
export function changeStartOfWeek(l: number) {
  const n = Math.min(6, Math.max(0, Math.floor(l)));
  changeGraphSetting("start_of_week", n, (next) => backend().setStartOfWeek(next), "start of week");
}

/** Persist new-page format and update graphMeta optimistically. Existing files
 * keep their format; failed writes roll back with a toast. */
export function changePreferredFormat(fmt: "md" | "org") {
  changeGraphSetting("preferred_format", fmt, (next) => backend().setPreferredFormat(next), "preferred page format");
}

const journalTitleFormatScope = {};
/** Apply the title format to UI state immediately, then start a backend
 * config write. Journal files are never renamed here (master e6f9b6e1ceae);
 * Settings proposes renames for title-named files. Return does not confirm
 * persistence. Failure may roll UI back and toasts. */
export function changeJournalTitleFormat(fmt: string) {
  const next = fmt.trim() || "MMM do, yyyy";
  const m = graphMeta();
  if (!m || m.journal_page_title_format === next) return;
  setGraphMeta({ ...m, journal_page_title_format: next });
  setJournalTitleFormat(next);
  bumpGraphEpoch(); // immediate: re-render open journal titles with the new format
  const owner = latestOwner(journalTitleFormatScope, "title", graphOwner(), () => graphMeta()?.root === m.root && graphMeta()?.journal_page_title_format === next);
  // Bump again once config.edn is written so the feed and the rename proposals
  // reload against the new format rather than racing the write.
  void writeOwned(owner, backend().setJournalTitleFormat(next, ["rename-page"]))
    .then((result) => {
      if (result.kind === "stale") return;
      bumpGraphEpoch();
      void refreshJournalConflicts(); // the queue surfaces any day the new format reveals
    })
    .catch((error) => {
      if (owner()) {
        setGraphMeta({ ...graphMeta()!, journal_page_title_format: m.journal_page_title_format });
        setJournalTitleFormat(m.journal_page_title_format);
        bumpGraphEpoch();
      }
      pushToast(`Could not save journal title format: ${String(error)}`, "error");
    });
}

export function journalMigrationSkipMessage(result: import("./types").JournalMigrationResult): string | null {
  const count = result.skipped.length;
  if (!count) return null;
  return `${count} journal file${count === 1 ? "" : "s"} skipped during migration: ${result.skipped.map(({ file, reason }) => `${file} (${reason})`).join("; ")}`;
}

// --- duplicate journal days (a date with >1 file, e.g. a date-stem file + a
// title-named one). The filename migration never clobbers, so these are left for
// the user to reconcile; we surface them rather than letting a day silently show
// twice in the feed. ---
export const [journalConflicts, setJournalConflicts] = createSignal<JournalConflict[]>([]);
// I-20: its reconcile actions take graph-relative paths; a switch empties it.
clearOnBindingInvalidated(() => setJournalConflicts([]));
/** Re-fetch the duplicate-journal-day list (Settings' fallback list and the
 *  in-page file rows). It no longer toasts: a duplicate day is a conflict-queue
 *  object, so it reaches the user through the badge, the overview and the day's
 *  own page like every other standing conflict (master 9dc54e4a7). */
export async function refreshJournalConflicts(): Promise<void> {
  const owner = graphOwner();
  try {
    const result = await readOwned(owner, backend().listJournalConflicts());
    if (result.kind === "stale") return;
    setJournalConflicts(result.value);
  } catch (error) {
    // A failed listing must not read as "no duplicate days": say so, but only
    // for the graph that asked (a switch already emptied the list).
    if (owner()) pushToast(`Could not check for duplicate journal days: ${String(error)}`, "error");
  }
}

// --- the Concord conflict inventory (src/conflictQueue.ts): sync-tool conflict
// copies, marker-bearing pages, and the derived queue over both. The calm
// sidebar badge, the Conflicts route and the in-page resolver carry the standing
// inventory; like master, only a copy that ARRIVES mid-session is announced. ---
/** Fetch the backend's derived conflict inventory (never stored). With
 *  `notify === "new"`, toast for sync copies that newly arrived. A failed read
 *  keeps the last successful inventory and reports its failure. */
export async function refreshSyncConflicts(notify: "new" | false = false): Promise<void> {
  const owner = graphOwner();
  const episode = beginConflictRefresh();
  try {
    const result = await readOwned(owner, backend().conflictInventory());
    if (result.kind === "stale" || !conflictRefreshCurrent(episode)) return;
    const previous = new Set(conflictQueue().map((conflict) => conflict.id));
    setConflictInventory(result.value);
    if (result.value.unreadable?.length) reportUiFailure("unreadable-files", result.value.unreadable.join(", "));
    const arrived = result.value.queue.filter((c) => c.source === "sync-copy" && !previous.has(c.id));
    if (notify === "new" && arrived.length) {
      const first = arrived[0];
      const toastId = pushToast(
        `${arrived.length} new sync conflict${arrived.length === 1 ? " needs" : "s need"} review`,
        "info",
        { sticky: true, action: { label: "Review", run: () => openPageTarget({ name: first.page_name, pageKind: first.kind, path: first.page_path }) },
          onDismiss: () => forgetArrivalNotice(toastId) }
      );
      trackArrivalNotice(toastId, arrived.map((conflict) => conflict.id));
    }
  } catch (error) {
    if (owner() && conflictRefreshCurrent(episode)) reportUiFailure("conflict-inventory", error);
  }
}

// --- which content pane is focused. Drives Ctrl+/- zoom routing (notes → whole
// interface, pdf → the PDF's own scale). Transient session state, not persisted. ---
export const [activePane, setActivePane] = createSignal<"notes" | "pdf">("notes");
let paneFocusSetter: ((paneId: string) => void) | undefined;
export function registerPaneFocusSetter(setter: (paneId: string) => void) {
  paneFocusSetter = setter;
}
/** Track the focused pane from clicks / focus moves. Capture-phase so it sees
 *  every interaction regardless of stopPropagation downstream. The notes pane is
 *  the default — anything outside the PDF pane (editor, sidebar, chrome) counts as
 *  "notes" for zoom purposes. Also owns one native launch-backup subscription,
 *  reports only this graph binding's failures, and disposes late registration.
 *  Cost O(1) per event, O(failures during graph open) on binding publication.
 *  Returns an uninstaller for both subscriptions. */
export function installPaneTracker(): () => void {
  let alive = true;
  let stopBackup: (() => void) | undefined;
  type BackupFailure = { bindingGeneration: number; failure: string };
  const [pendingBackup, setPendingBackup] = createSignal<BackupFailure[]>([]);
  const stopPending = createRoot((dispose) => {
    createEffect(() => {
      const pending = pendingBackup();
      if (graphTransitioning() || pending.length === 0) return;
      const generation = backend().graphBindingGeneration();
      setPendingBackup([]);
      for (const payload of pending) {
        if (payload.bindingGeneration === generation) reportUiFailure("backup-read", payload.failure);
      }
    });
    return dispose;
  });
  if (isTauri()) void readOwnedResource(ownedWhen(() => alive),
    listen<BackupFailure>("backup-failed", ({ payload }) => {
      if (!alive) return;
      if (payload.bindingGeneration === backend().graphBindingGeneration()) reportUiFailure("backup-read", payload.failure);
      // Native backup can fail before load_graph's reply publishes its binding.
      else if (graphTransitioning()) setPendingBackup((pending) => [...pending, payload]);
    }), (stop) => stop(),
  ).then((result) => {
    if (result.kind === "current") { if (alive) stopBackup = result.value; else result.value(); }
  })
    .catch((error) => { if (alive) reportUiFailure("backup-feedback", error); });
  const update = (e: Event) => {
    const t = e.target as Element | null;
    const container = t?.closest?.("[data-pane-id]") ?? null;
    // Focus landing OUTSIDE any pane (the Ctrl+K / palette input, dialogs —
    // they are global overlays) must NOT steal pane focus: the overlay is
    // pane-neutral and the command it runs targets focusedPaneId(). The old
    // "?? main" default reset pane focus on EVERY switcher open, so palette
    // splits and Ctrl+K picks always landed in "main" (Martin's Jul 8
    // wrong-pane report). Deliberate pointer clicks outside keep the old
    // main default.
    if (e.type === "focusin" && !container) return;
    const paneId = container?.getAttribute("data-pane-id") ?? "main";
    paneFocusSetter?.(paneId);
    setActivePane(paneId === "pdf" ? "pdf" : "notes");
  };
  const pointerdown = (e: Event) => {
    // Any click exits pane-select (standard modal behavior); without this the
    // mode goes stale — the ring lingers and, worse, its keyboard handler
    // would still be armed after the user clicks off to do something else.
    exitPaneSelect();
    // A middle-click is a background-tab gesture, not a pane activation. Keep
    // the pane that was already active so links in another pane or the sidebar
    // open their background tab there (GH #87). Target anchors also prevent the
    // middle-button default focus so a following focusin cannot undo this.
    if ("button" in e && (e as PointerEvent).button === 1) return;
    // Global chrome can still target the pane the user last focused. Back,
    // Forward, Search, and Journals all call the focused-router facade; letting
    // this capture-phase pointer event fall through would retarget to `main`
    // before their click handler runs, making right-pane history look dead.
    // Ordinary outside-pane clicks keep the deliberate main fallback below.
    const target = e.target as Element | null;
    if (target?.closest?.("[data-pane-focus-neutral]")) return;
    update(e);
  };
  window.addEventListener("pointerdown", pointerdown, true);
  window.addEventListener("focusin", update, true);
  return () => {
    alive = false;
    stopBackup?.();
    stopPending();
    window.removeEventListener("pointerdown", pointerdown, true);
    window.removeEventListener("focusin", update, true);
  };
}

// --- focus mode (hide chrome + fullscreen) + dim-inactive-blocks ---
// Focus is a deliberate session mode → NOT persisted. Dim is an appearance
// preference → persisted, like wide/document. Focus composes with wide/document:
// it only hides the sidebars + topbar and goes fullscreen; it doesn't touch the
// content width or your other layout toggles.
export const [focusMode, setFocusMode] = createSignal(false);

const DIM_KEY = "logseq-claude.dim";
export const [dimInactiveBlocks, setDimInactiveBlocks] = createSignal(loadStr(DIM_KEY) === "1");
export function toggleDimInactiveBlocks() {
  const v = !dimInactiveBlocks();
  setDimInactiveBlocks(v);
  saveStr(DIM_KEY, v ? "1" : null);
}

// When on (default), entering focus mode auto-enables dim-inactive-blocks and
// exiting restores the prior dim state. The override is transient (it doesn't
// rewrite the persisted dim preference, so your manual `t b` choice is kept).
const DIM_IN_FOCUS_KEY = "logseq-claude.dimInFocus";
export const [dimInFocus, setDimInFocusSig] = createSignal(loadStr(DIM_IN_FOCUS_KEY) !== "0");
export function setDimInFocus(v: boolean) {
  setDimInFocusSig(v);
  saveStr(DIM_IN_FOCUS_KEY, v ? null : "0");
}
export function toggleDimInFocus() {
  setDimInFocus(!dimInFocus());
}

// --- carry-unfinished-tasks settings (persisted) ---
const CARRY_CTX_KEY = "logseq-claude.carryKeepsContext";
const CARRY_HDR_KEY = "logseq-claude.carryHeader";
const CARRY_N_KEY = "logseq-claude.carryDays";
// Default ON: move whole top-level blocks that contain an open task (keep their
// context), rather than pulling just the task out.
export const [carryKeepsContext, setCarryKeepsContextSig] = createSignal(loadStr(CARRY_CTX_KEY) !== "0");
export function setCarryKeepsContext(v: boolean) {
  setCarryKeepsContextSig(v);
  saveStr(CARRY_CTX_KEY, v ? null : "0");
}
// Default OFF: prepend a "Carried over" header above the carried blocks.
export const [carryHeader, setCarryHeaderSig] = createSignal(loadStr(CARRY_HDR_KEY) === "1");
export function setCarryHeader(v: boolean) {
  setCarryHeaderSig(v);
  saveStr(CARRY_HDR_KEY, v ? "1" : null);
}
/** Header text to insert, or null when disabled. */
export function carryHeaderText(): string | null {
  return carryHeader() ? "Carried over" : null;
}
// Show the carry-over action buttons on journal titles (default ON). Some
// people find them disruptive — turn off to fall back to the right-click menu.
const CARRY_BTNS_KEY = "logseq-claude.showCarryButtons";
export const [showCarryButtons, setShowCarryButtonsSig] = createSignal(loadStr(CARRY_BTNS_KEY) !== "0");
export function setShowCarryButtons(v: boolean) {
  setShowCarryButtonsSig(v);
  saveStr(CARRY_BTNS_KEY, v ? null : "0");
}
// N for the "carry last N days" command (presets 7/30/365 don't use it).
export const [carryDays, setCarryDaysSig] = createSignal(Number(loadStr(CARRY_N_KEY)) || 7);
export function setCarryDays(n: number) {
  const v = Math.max(1, Math.min(3650, Math.floor(n) || 7));
  setCarryDaysSig(v);
  saveStr(CARRY_N_KEY, String(v));
}

// --- journal agenda ("Scheduled & Deadline") window (persisted) ---
// How many days BACK and AHEAD of today an item's SCHEDULED/DEADLINE date may be
// before it drops out of the agenda. Default 7/7 (the historical hard-coded
// window). The window is tested against the scheduled/deadline date itself —
// NOT the journal day the item happens to live on — and the query scans the
// whole graph, so an overdue item on an old page still shows while it's in range.
const AGENDA_BACK_KEY = "logseq-claude.agendaDaysBack";
const AGENDA_AHEAD_KEY = "logseq-claude.agendaDaysAhead";
function loadDays(key: string, def: number): number {
  // An UNSET key must fall back to `def` — not 0. (Number(null) and Number("")
  // are both 0, which silently turned the unset 7/7 agenda window into 0/0.)
  const raw = loadStr(key);
  if (raw === null || raw.trim() === "") return def;
  const n = Number(raw);
  return Number.isFinite(n) && n >= 0 ? n : def;
}
export const [agendaDaysBack, setAgendaDaysBackSig] = createSignal(loadDays(AGENDA_BACK_KEY, 7));
export const [agendaDaysAhead, setAgendaDaysAheadSig] = createSignal(loadDays(AGENDA_AHEAD_KEY, 7));
export function setAgendaDaysBack(n: number) {
  const v = Math.max(0, Math.min(3650, Math.floor(n) || 0));
  setAgendaDaysBackSig(v);
  saveStr(AGENDA_BACK_KEY, String(v));
}
export function setAgendaDaysAhead(n: number) {
  const v = Math.max(0, Math.min(3650, Math.floor(n) || 0));
  setAgendaDaysAheadSig(v);
  saveStr(AGENDA_AHEAD_KEY, String(v));
}
/**
 * The journal agenda's query DSL, built from the configured window. Matches a
 * block if its SCHEDULED date OR its DEADLINE date falls in
 * [today − back, today + ahead] — keyed off the date itself, not the page's
 * journal day, so a stale-deadline item on a recent day no longer shows and an
 * overdue item on an old page still does.
 *
 * Finished tasks are excluded: OG's `get-date-scheduled-or-deadlines` drops
 * DONE/CANCELED/CANCELLED markers (db/model.cljs), so the agenda only lists work
 * still to do. A scheduled item with no marker at all is kept (matches OG's
 * `:block/marker "NIL"` default).
 */
export function agendaQuery(): string {
  const lo = `-${agendaDaysBack()}d`;
  const hi = `+${agendaDaysAhead()}d`;
  const window = `(or (between scheduled ${lo} ${hi}) (between deadline ${lo} ${hi}))`;
  return `query (and ${window} (not (task DONE CANCELED CANCELLED)))`;
}

// Block id of a "/Query" block whose QueryBuilder opens its add-filter picker once on mount.
export const [queryBuilderAutoOpen, setQueryBuilderAutoOpen] = graphScopedSignal<string>();
/** One graph-bound reviewed query export; a graph switch closes the dialog. */
export const [queryExportRequest, setQueryExportRequest] = graphScopedSignal<import("./types").QueryPublicationRequest>();
export function openQueryExport(request: import("./types").QueryPublicationRequest): void { setQueryExportRequest(request); }
export function closeQueryExport(): void { setQueryExportRequest(null); }

export type PropsPanelScope = { kind: "page"; name: string } | { kind: "block"; id: string };
/** The one open properties panel (GH #164) or null; page OR block scope despite the name (`name` = exact store page name, `id` = in-memory
 *  block id), at viewport x,y. open*Props replaces any open panel. `binding` = graph session at open: a graph switch closes the panel,
 *  and after a store reset or a reload of its page/block the panel refuses every write visibly and closes (PageProps.tsx writeOne).
 *  An unknown/unloaded block id or page opens a read-only notice, never an edit row. Nothing here persists; O(1). */
export const [pagePropsPanel, setPagePropsPanel] = createSignal<{ scope: PropsPanelScope; x: number; y: number; binding: ReturnType<typeof captureBinding> } | null>(null);
export function openPageProps(name: string, x: number, y: number) {
  setPagePropsPanel({ scope: { kind: "page", name }, x, y, binding: captureBinding() });
}
export function openBlockProps(id: string, x: number, y: number) {
  setPagePropsPanel({ scope: { kind: "block", id }, x, y, binding: captureBinding() });
}
export function closePageProps() {
  setPagePropsPanel(null);
}

// "Copy / export as" modal — a live-preview text export of a block subtree or a
// multi-block selection, with indent-style + remove options (mirrors OG Logseq).
export type ExportRequest = { ids: string[] } | { nodes: import("./editor/exportText").ExportNode[]; count: number };
export const [exportModal, setExportModal] = graphScopedSignal<ExportRequest>();
/** Open the shared export modal for a selection of document block ids. */
export function openExportModal(ids: string[]) {
  if (ids.length) setExportModal({ ids });
}
/** Open the shared export modal for an already materialized, read-only forest.
 * The caller supplies its visible block count; this does not write graph data. */
export function openExportNodesModal(nodes: import("./editor/exportText").ExportNode[], count: number) {
  if (nodes.length) setExportModal({ nodes, count });
}
export function closeExportModal() {
  setExportModal(null);
}

export function toggleFocusMode() {
  if (focusMode()) void exitFocusMode();
  else void enterFocusMode();
}
/** Enable focus and request fullscreen. Failure toasts without rollback; resolves after request. O(1) plus native latency. */
export async function enterFocusMode() {
  if (focusMode()) return;
  // When the setting is on, focus mode owns dim: on while focused, off when
  // exited (a transient signal change — it doesn't rewrite the t-b preference).
  if (dimInFocus()) setDimInactiveBlocks(true);
  setFocusMode(true);
  try {
    await setFocusFullscreen(true);
  } catch {
    pushToast("Could not enter fullscreen focus mode.", "error");
  }
}
/** Disable focus and request fullscreen exit. Failure toasts without rollback; resolves after request. O(1) plus native latency. */
export async function exitFocusMode() {
  if (!focusMode()) return;
  setFocusMode(false);
  if (dimInFocus()) setDimInactiveBlocks(false);
  try {
    await setFocusFullscreen(false);
  } catch {
    pushToast("Could not leave fullscreen focus mode.", "error");
  }
}

/** Toggle the resolved palette between Light and Dark. A System choice becomes
 * the opposite manual palette; persistence and native appearance follow. */
export function toggleTheme() {
  setAppearancePreference(theme() === "light" ? "dark" : "light");
}

// Left sidebar open/collapsed — persisted (default open; store only when collapsed).
const SIDEBAR_OPEN_KEY = "logseq-claude.sidebarOpen";
export const [sidebarOpen, setSidebarOpen] = createSignal(loadStr(SIDEBAR_OPEN_KEY) !== "0");
export const [favoritesSectionExpanded, setFavoritesSectionExpanded] = createSignal(true);
export const [recentSectionExpanded, setRecentSectionExpanded] = createSignal(true);

export function toggleFavoritesSection() {
  setFavoritesSectionExpanded((open) => !open);
  scheduleSessionSave();
}

export function toggleRecentSection() {
  setRecentSectionExpanded((open) => !open);
  scheduleSessionSave();
}

/** Reset graph-scoped disclosure preferences before another graph's persisted
 * session is restored. A legacy/missing session therefore defaults expanded
 * instead of inheriting the graph that was open previously. */
export function resetLeftSidebarSections() {
  setFavoritesSectionExpanded(true);
  setRecentSectionExpanded(true);
}

function persistLeftOpen(v: boolean) {
  if (!saveStr(SIDEBAR_OPEN_KEY, v ? null : "0")) return;
  setSidebarOpen(v);
  scheduleSessionSave(); // durable open/closed state (localStorage isn't kept)
}
export function setLeftSidebarOpen(v: boolean, trigger?: HTMLElement | null) {
  if (v && mobileDrawerMode()) {
    setRightSidebarOpen(false);
    captureDrawerOpener(trigger);
  }
  persistLeftOpen(v);
}
export function toggleSidebar(trigger?: HTMLElement | null) {
  setLeftSidebarOpen(!sidebarOpen(), trigger);
}

/** Apply sidebar open/closed + right-sidebar items restored from the persisted
 *  session (router.restoreSession). Sets the signals directly — no save trigger,
 *  so restoring can't loop back into another save. */
export interface SidebarSessionState {
  left?: boolean;
  right?: boolean;
  items?: SidebarItem[];
  favoritesExpanded?: boolean;
  recentExpanded?: boolean;
}

export function applySidebarSession(s: SidebarSessionState) {
  if (typeof s.left === "boolean") setSidebarOpen(s.left);
  const items = Array.isArray(s.items) ? s.items.filter(validSidebarItem) : undefined;
  // Session application can replace the mounted collection before it changes
  // the open signal. Prepare the old surfaces first, while their textarea blur
  // handlers and controller ownership are still live.
  const collectionPrepared = !!items
    && rightSidebarOpen()
    && !sameSidebarItems(rightSidebar(), items)
    && prepareRightSidebarMountedSurfaces();
  if (items) setRightSidebarRaw(items);
  // Session restoration is also a whole-panel close path, but must not schedule
  // another session save. Keep it on the same visibility boundary as scrim,
  // Escape, Back, toolbar, and explicit close instead of writing the signal.
  if (typeof s.right === "boolean") {
    setRightSidebarOpenState(s.right, { persist: false, prepared: collectionPrepared });
  }
  setFavoritesSectionExpanded(s.favoritesExpanded ?? true);
  setRecentSectionExpanded(s.recentExpanded ?? true);
  normalizeSidebarDrawers();
}

export { sidebarWidth, setSidebarWidth, persistSidebarWidth, rightSidebarWidth, setRightSidebarWidth, persistRightSidebarWidth } from "./sidebarSizing";

// Favorites live in ./favorites (one arrangement tree, one identity key).
export { favorites, favoriteKey, isFavorite, seedFavorites, setFavorites, toggleFavorite, type FavItem } from "./favorites";
/** Remove navigation entries for a deleted target. Favorites match by
 * favoriteKey (kind + page identity); recents and sidebar check kind/path.
 * Favorite changes queue config write; others schedule session persistence.
 * O(favorites + recents + sidebar items). */
export function removeDeletedPageFromNavigation(target: PageTarget): void;
export function removeDeletedPageFromNavigation(name: string, kind: PageKind): void;
export function removeDeletedPageFromNavigation(targetOrName: PageTarget | string, kind?: PageKind) {
  const target: PageTarget = typeof targetOrName === "string"
    ? { name: targetOrName, pageKind: kind! }
    : targetOrName;
  const name = target.name;
  kind = target.pageKind;
  forgetDeletedFavorite(name, kind);

  const nextRecents = recentPages().filter((r) => !(
    r.name === name && r.kind === kind && (target.path === undefined || r.path === target.path)
  ));
  if (nextRecents.length !== recentPages().length) {
    setRecentPages(nextRecents);
    scheduleSessionSave();
  }

  const nextSidebar = rightSidebar().filter((item) => {
    const sameLogical = item.kind === "page"
      ? item.name === name && item.pageKind === kind
      : item.page === name && item.pageKind === kind;
    return !sameLogical || (target.path !== undefined && item.path !== target.path);
  });
  if (nextSidebar.length !== rightSidebar().length) setRightSidebar(nextSidebar);
}

/** After backend rename, re-key favorite, recent and sidebar entries in order,
 * deduplicating destinations even if source is absent. Only changed stores
 * schedule persistence; writes need not finish before a later openPage. Does
 * not rename or open a page. O(favorites + recents² + sidebar items). */
export function renamePageInNavigation(from: PageTarget, to: PageTarget, opts?: { favorites?: boolean }): void;
export function renamePageInNavigation(from: string, to: string, opts?: { favorites?: boolean }): void;
export function renamePageInNavigation(fromOrName: PageTarget | string, toOrName: PageTarget | string, opts: { favorites?: boolean } = {}) {
  const from: PageTarget = typeof fromOrName === "string"
    ? { name: fromOrName, pageKind: "page" }
    : fromOrName;
  const to: PageTarget = typeof toOrName === "string"
    ? { name: toOrName, pageKind: from.pageKind }
    : toOrName;
  if (opts.favorites !== false) renameFavorite(from, to);

  const nextRecents = recentPages().reduce<RecentItem[]>((out, item) => {
    const matches = item.kind === from.pageKind && item.name === from.name
      && (from.path === undefined || item.path === from.path);
    const next = matches
      ? { name: to.name, kind: to.pageKind, ...(to.path ? { path: to.path } : {}) }
      : item;
    if (!out.some((seen) => seen.kind === next.kind && seen.name === next.name)) out.push(next);
    return out;
  }, []);
  if (nextRecents.some((item, i) => item !== recentPages()[i]) || nextRecents.length !== recentPages().length) {
    setRecentPages(nextRecents);
    scheduleSessionSave();
  }

  const seenSidebar = new Set<string>();
  const nextSidebar: SidebarItem[] = [];
  for (const item of rightSidebar()) {
    const matches = item.kind === "page"
      ? item.name === from.name && item.pageKind === from.pageKind && (from.path === undefined || item.path === from.path)
      : item.page === from.name && item.pageKind === from.pageKind && (from.path === undefined || item.path === from.path);
    const next: SidebarItem = !matches ? item : item.kind === "page"
      ? { ...item, name: to.name, pageKind: to.pageKind, path: to.path }
      : { ...item, page: to.name, pageKind: to.pageKind, path: to.path };
    const key = sidebarItemKey(next);
    if (seenSidebar.has(key)) continue;
    seenSidebar.add(key);
    nextSidebar.push(next);
  }
  if (nextSidebar.some((item, i) => item !== rightSidebar()[i]) || nextSidebar.length !== rightSidebar().length) {
    setRightSidebar(nextSidebar);
  }
}
/** A load resolved the requested page name `from` to the backend's page `to`
 * (a case variant, or an alias's owner). Only the views (Recent, sidebar)
 * follow; opening a page never rewrites the favorites config (D11), and a case
 * variant is already the same favorite by `favoriteKey`. */
export function adoptResolvedPageName(from: string, to: string): void {
  renamePageInNavigation(from, to, { favorites: false });
}
// Recently-visited pages (navigation history), newest first. Unlike Favorites,
// Recent is graph-scoped session state and may retain one exact physical owner.
const RECENT_KEY = "logseq-claude.recent";
export interface RecentItem {
  name: string;
  kind: PageKind;
  path?: string;
}
function sanitizeRecent(value: unknown): RecentItem[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((entry): RecentItem[] => {
    if (!entry || typeof entry !== "object") return [];
    const item = entry as Record<string, unknown>;
    if (typeof item.name !== "string" || item.name.length > 4096) return [];
    if (item.kind !== "page" && item.kind !== "journal") return [];
    if (item.path !== undefined && (typeof item.path !== "string" || item.path.length > 4096)) return [];
    return [{ name: item.name, kind: item.kind, ...(item.path ? { path: item.path } : {}) }];
  }).slice(0, 20);
}
function loadLegacyRecent(): RecentItem[] {
  try {
    return sanitizeRecent(JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]"));
  } catch {
    return [];
  }
}
let legacyRecent = loadLegacyRecent();
export const [recentPages, setRecentPages] = createSignal<RecentItem[]>([]);
export function legacyRecentPages(): RecentItem[] {
  return legacyRecent.map((item) => ({ ...item }));
}
export function clearLegacyRecentSource() {
  legacyRecent = [];
  try { localStorage.removeItem(RECENT_KEY); } catch { /* ignore */ }
}
export { sanitizeRecent };
/** Drop the recent-pages list. Called on a graph SWITCH so the previous graph's
 *  pages don't linger in quick-switch (Ctrl-K) / the sidebar "recent" list. */
export function clearRecent() {
  setRecentPages([]);
}
export function pushRecent(target: PageTarget): void;
export function pushRecent(name: string, kind?: PageKind): void;
export function pushRecent(targetOrName: PageTarget | string, kind: PageKind = "page") {
  const target: RecentItem = typeof targetOrName === "string"
    ? { name: targetOrName, kind }
    : { name: targetOrName.name, kind: targetOrName.pageKind, ...(targetOrName.path ? { path: targetOrName.path } : {}) };
  const first = recentPages()[0];
  if (first?.name === target.name && first.kind === target.kind && first.path === target.path) return;
  // One visually indistinguishable same-name row: the latest selected physical
  // owner replaces the previous one rather than leaving duplicate labels.
  const cur = recentPages().filter((r) => !(r.name === target.name && r.kind === target.kind));
  const next = [target, ...cur].slice(0, 20);
  setRecentPages(next);
  scheduleSessionSave();
}

// User keyboard-shortcut overrides set from the Settings modal. Persisted
// locally and layered on top of config.edn `:shortcuts` (which is itself on top
// of the built-in defaults). Map of command id -> binding string.
const SHORTCUTS_KEY = "logseq-claude.shortcuts";
function loadShortcutOverrides(): Record<string, string> {
  try {
    const v = JSON.parse(localStorage.getItem(SHORTCUTS_KEY) ?? "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}
export const [shortcutOverrides, setShortcutOverrides] =
  createSignal<Record<string, string>>(loadShortcutOverrides());
function persistShortcuts(next: Record<string, string>) {
  // Like changeAccent: a refused write is announced and NOT applied, so the
  // shortcut the user sees is the one that will still be there after a restart.
  if (!saveStr(SHORTCUTS_KEY, JSON.stringify(next), "keyboard shortcuts")) return;
  setShortcutOverrides(next);
}
export function setShortcutOverride(id: string, binding: string) {
  persistShortcuts({ ...shortcutOverrides(), [id]: binding });
}
export function resetShortcutOverride(id: string) {
  const next = { ...shortcutOverrides() };
  delete next[id];
  persistShortcuts(next);
}

// Date picker: planning, journal-link insertion, and typed sheet date properties.
// Journal callback receives the configured title only on commit, never cancellation.
export type DatePickerTarget =
  | "scheduled"
  | "deadline"
  | { insertJournal: (title: string) => void }
  | { field: `prop:${string}`; fieldType: "date" | "datetime" };
export const [datePicker, setDatePicker] = graphScopedSignal<
  { blockId: string; which: DatePickerTarget; x: number; y: number }
>();
export function openDatePicker(blockId: string, which: DatePickerTarget, x: number, y: number) {
  setDatePicker({ blockId, which, x, y });
}
export function closeDatePicker() {
  setDatePicker(null);
}

// Sheet formula/filter editor popup. Mounted at the app root like DatePicker so
// table/board menus can open one positioned editor without owning popup state.
export type FormulaEditorHome = { kind: "block"; id: string } | { kind: "page"; name: string };
export type FormulaEditorMode = "add" | "edit" | "filter";
export interface FormulaEditorTarget {
  mode: FormulaEditorMode;
  ownerId: string;
  schemaPage?: string;
  x: number;
  y: number;
  name?: string;
  expr: string;
  formulas: readonly [string, string][];
  fields: readonly string[];
  home?: FormulaEditorHome | null;
}
/** Graph-scoped (I-20): a graph switch closes the editor; `save` refuses a stale target. */
export const [formulaEditor, setFormulaEditor] = graphScopedSignal<FormulaEditorTarget>();
export function openFormulaEditor(target: FormulaEditorTarget) {
  setFormulaEditor(target);
}
export function closeFormulaEditor() {
  setFormulaEditor(null);
}

// Block zoom: focus a single block's subtree (click its bullet). Zoom is part of
// the active tab's ROUTE (the block's stable uuid) — so it's per-tab, joins the
// back/forward history, and a block can be opened pre-zoomed in its own tab
// (middle-click a bullet). zoomedBlock derives from the current route.
export function zoomedBlock(): string | null {
  const r = useContext(PaneContext)?.router.route() ?? route();
  return r.kind === "page" ? r.block ?? null : null;
}
export function zoomInto(id: string) {
  focusBlock(id);
}
export function zoomOut() {
  focusBlock(null);
}

// Right sidebar: a stack of items opened for reference (shift-click anything).
//
// "Everything is a block": every reference resolves to one of two universal
// targets — a *page* (by name) or a *block* (by stable uuid). Both are LIVE
// references, not snapshots: the sidebar loads the target's page into the shared
// working set and renders the same editable <Block> the main view uses, so an
// edit in the sidebar is an edit to the one underlying node and shows up
// everywhere (OG's single-source-of-truth model, kept lazy).
export interface SidebarPage {
  kind: "page";
  name: string;
  pageKind: "journal" | "page";
  /** Exact graph-relative owner. Absent on legacy entries, which resolve by name. */
  path?: string;
  collapsed?: boolean;
}
export interface SidebarBlock {
  kind: "block";
  uuid: string; // stable block id — the live handle into the store
  page: string; // the page it lives on (loaded on demand)
  pageKind: "journal" | "page";
  /** Exact graph-relative owner. Absent on legacy entries, which resolve by name. */
  path?: string;
  /** Sibling-index path to an ID-less block, saved by a session instead of writing an
   * `id::` (navigation never mutates the graph); settled into `uuid` once resolved. */
  blockPos?: number[];
  collapsed?: boolean;
}
export type SidebarItem = SidebarPage | SidebarBlock;

export interface HistorySidebarContext {
  open: boolean;
  items: SidebarItem[];
}

/** Stable presentation identity for one sidebar collection item. Unlike an
 * array index, it survives closing a neighbor and page renames are re-keyed by
 * renamePageInNavigation. */
export function sidebarItemKey(item: SidebarItem): string {
  return item.kind === "page"
    ? `page:${item.pageKind}:${item.name}`
    : `block:${item.uuid}`;
}

function sameBlockPos(a: readonly number[] | undefined, b: readonly number[] | undefined): boolean {
  return a === b || (!!a && !!b && a.length === b.length && a.every((n, i) => n === b[i]));
}

function sameSidebarItems(left: readonly SidebarItem[], right: readonly SidebarItem[]): boolean {
  return left.length === right.length && left.every((item, index) => {
    const other = right[index];
    if (!other || item.kind !== other.kind || item.collapsed !== other.collapsed) return false;
    if (item.kind === "page" && other.kind === "page") {
      return item.name === other.name && item.pageKind === other.pageKind && item.path === other.path;
    }
    return item.kind === "block" && other.kind === "block"
      && item.uuid === other.uuid && item.page === other.page && item.pageKind === other.pageKind
      && item.path === other.path && sameBlockPos(item.blockPos, other.blockPos);
  });
}

// What's open in the right sidebar — persisted across restarts. Items are plain
// JSON (page name / block uuid). Page items always restore; block items resolve
// only if their block still carries a stable uuid (a ref target with `id::`) —
// `pruneSidebarBlocks` drops the rest after the graph loads (see graph.ts), so
// stale entries don't linger.
const RS_ITEMS_KEY = "logseq-claude.rightSidebarItems";
function validSidebarItem(i: unknown): i is SidebarItem {
  if (!i || typeof i !== "object") return false;
  const o = i as Record<string, unknown>;
  if (o.collapsed !== undefined && typeof o.collapsed !== "boolean") return false;
  if (o.path !== undefined && typeof o.path !== "string") return false;
  if (o.kind === "page") return typeof o.name === "string";
  if (o.kind === "block") {
    return typeof o.uuid === "string" && typeof o.page === "string"
      && (o.blockPos === undefined || parseBlockPos(o.blockPos) !== null);
  }
  return false;
}
export function parseStoredSidebarItems(raw: string | null): SidebarItem[] {
  try {
    const arr = raw ? JSON.parse(raw) : [];
    return Array.isArray(arr) ? arr.filter(validSidebarItem) : [];
  } catch {
    return [];
  }
}
function loadRsItems(): SidebarItem[] {
  try {
    return parseStoredSidebarItems(localStorage.getItem(RS_ITEMS_KEY));
  } catch {
    return [];
  }
}
const [rightSidebar, setRightSidebarRaw] = createSignal<SidebarItem[]>(loadRsItems());
export { rightSidebar };

// The right sidebar has its own open/closed state (persisted), independent of
// whether it currently holds items — so it can be toggled (icon / `t r`) and
// shows an empty hint when open but empty. Opening an item forces it open.
const RS_OPEN_KEY = "logseq-claude.rightSidebarOpen";
// Open if explicitly persisted open, or (migration / first run) if items were
// restored — so a populated sidebar shows even before the open-state was tracked.
const [rightSidebarOpen, setRightSidebarOpenSignal] = createSignal(
  loadStr(RS_OPEN_KEY) === "1" || rightSidebar().length > 0
);
export { rightSidebarOpen };
let prepareRightSidebarClose: (() => void) | undefined;
let preparingRightSidebarClose = false;
/** RightSidebar installs its real editor/surface teardown here.  Every whole
 * panel close (mobile scrim/Escape/Back and desktop toolbar) uses this seam. */
export function registerRightSidebarClosePreparation(prepare: () => void): () => void {
  prepareRightSidebarClose = prepare;
  return () => { if (prepareRightSidebarClose === prepare) prepareRightSidebarClose = undefined; };
}

/** Invoke the mounted RightSidebar's real blur/controller teardown exactly
 * before a visibility or collection transition can unmount its surfaces. */
function prepareRightSidebarMountedSurfaces(): boolean {
  const prepare = prepareRightSidebarClose;
  if (!prepare || preparingRightSidebarClose) return false;
  preparingRightSidebarClose = true;
  try {
    prepare();
    return true;
  } finally {
    preparingRightSidebarClose = false;
  }
}

export function activeDrawer(): DrawerSide | null {
  if (!mobileDrawerMode()) return null;
  if (rightSidebarOpen()) return "right";
  return sidebarOpen() ? "left" : null;
}

function setRightSidebarOpenState(
  v: boolean,
  options: { trigger?: HTMLElement | null; persist?: boolean; prepared?: boolean } = {}
): boolean {
  if (rightSidebarOpen() === v) return false;
  if (!v && !options.prepared) prepareRightSidebarMountedSurfaces();
  if (v && mobileDrawerMode()) {
    persistLeftOpen(false);
    captureDrawerOpener(options.trigger);
  }
  setRightSidebarOpenSignal(v);
  if (options.persist !== false) {
    saveStr(RS_OPEN_KEY, v ? "1" : null);
    scheduleSessionSave(); // durable open/closed state (localStorage isn't kept)
  }
  return true;
}
function setRightSidebarOpenRaw(v: boolean, trigger?: HTMLElement | null): boolean {
  return setRightSidebarOpenState(v, { trigger });
}
export function setRightSidebarOpen(v: boolean, trigger?: HTMLElement | null) {
  setRightSidebarOpenRaw(v, trigger);
}
/** Shared, idempotent whole-panel close boundary. */
export function closeRightSidebarSafely() {
  return setRightSidebarOpenRaw(false);
}
export function toggleRightSidebar(trigger?: HTMLElement | null) {
  setRightSidebarOpenRaw(!rightSidebarOpen(), trigger);
}
export function setRightSidebar(items: SidebarItem[]) {
  // Graph transitions and close-all clear the mounted collection without
  // necessarily closing the panel. They share the same pre-unmount boundary;
  // a preceding explicit preparation is harmless because editor teardown is
  // idempotent and the second pass sees no live edit owner.
  if (rightSidebarOpen() && rightSidebar().length > 0 && items.length === 0) {
    prepareRightSidebarMountedSurfaces();
  }
  setRightSidebarRaw(items);
  try {
    if (items.length) localStorage.setItem(RS_ITEMS_KEY, JSON.stringify(items));
    else localStorage.removeItem(RS_ITEMS_KEY);
  } catch {
    // ignore
  }
  scheduleSessionSave(); // durable right-sidebar items (localStorage isn't kept)
}

/** History captures the same sidebar-open/item app state as OG does at
 * `src/main/frontend/modules/editor/undo_redo.cljs:261-272`
 * (OG commit 6e7afa8eb). */
export function captureHistorySidebarContext(): HistorySidebarContext {
  return { open: rightSidebarOpen(), items: rightSidebar().map((item) => ({ ...item })) };
}

export function restoreHistorySidebarContext(context: HistorySidebarContext) {
  const items = context.items.filter(validSidebarItem).map((item) => ({ ...item }));
  setRightSidebar(items);
  setRightSidebarOpen(context.open);
}

export function openPageInSidebar(target: PageTarget): void;
export function openPageInSidebar(name: string, pageKind?: PageKind, path?: string): void;
export function openPageInSidebar(
  targetOrName: PageTarget | string,
  pageKind: PageKind = "page",
  path?: string,
) {
  let { name, pageKind: kind, path: targetPath } = typeof targetOrName === "string"
    ? { name: targetOrName, pageKind, path }
    : targetOrName;
  pageKind = kind;
  path = targetPath;
  if (pageKind === "page" && !path) name = navigationName(name);
  setRightSidebarOpen(true);
  // The working set is intentionally name-keyed. A duplicate physical file with
  // the same logical page name therefore replaces that one live sidebar slot;
  // retaining both would render/save one of them through the other's identity.
  const existing = rightSidebar().findIndex((i) =>
    i.kind === "page" && i.name === name && i.pageKind === pageKind
  );
  if (existing >= 0) {
    const replaced = rightSidebar().map((item, i) =>
      i === existing ? { ...item, name, pageKind, path, collapsed: false } : item
    );
    setRightSidebar(replaced.filter((item, i) => {
      if (i === existing) return true;
      const sameOwnerName = item.kind === "page"
        ? item.name === name && item.pageKind === pageKind
        : item.page === name && item.pageKind === pageKind;
      return !sameOwnerName || item.path === path;
    }));
    return;
  }
  const compatible = rightSidebar().filter((item) => {
    const sameOwnerName = item.kind === "page"
      ? item.name === name && item.pageKind === pageKind
      : item.page === name && item.pageKind === pageKind;
    return !sameOwnerName || item.path === path;
  });
  setRightSidebar([{ kind: "page", name, pageKind, path }, ...compatible]);
}
export function openBlockInSidebar(ref: {
  uuid: string;
  page: string;
  pageKind: "journal" | "page";
  path?: string;
}) {
  setRightSidebarOpen(true);
  const existing = rightSidebar().findIndex((i) => i.kind === "block" && i.uuid === ref.uuid);
  if (existing >= 0) {
    const updated = rightSidebar().map((item, i) =>
      i === existing ? { ...item, ...ref, collapsed: false } : item
    );
    setRightSidebar(updated.filter((item, i) => {
      if (i === existing) return true;
      const sameOwnerName = item.kind === "page"
        ? item.name === ref.page && item.pageKind === ref.pageKind
        : item.page === ref.page && item.pageKind === ref.pageKind;
      return !sameOwnerName || item.path === ref.path;
    }));
    return;
  }
  // A pathful duplicate selection must evict incompatible same-name items. The
  // shared store can retain one physical owner for a logical page name, so stale
  // siblings would otherwise become misleading missing/wrong-file block views.
  const compatible = rightSidebar().filter((item) => {
    const sameOwnerName = item.kind === "page"
      ? item.name === ref.page && item.pageKind === ref.pageKind
      : item.page === ref.page && item.pageKind === ref.pageKind;
    if (!sameOwnerName) return true;
    const itemPath = item.path;
    return itemPath === ref.path;
  });
  setRightSidebar([{ kind: "block", ...ref }, ...compatible]);
}

/** Restored desktop state can contain both panels.  Entering drawer mode has a
 * deliberate winner: the contextual right sidebar. */
export function normalizeSidebarDrawers() {
  if (!mobileDrawerMode()) {
    // A compact opener must not survive a resize into persistent-sidebar mode
    // and later steal focus after an unrelated programmatic drawer open.
    clearDrawerOpener();
    return;
  }
  if (rightSidebarOpen()) {
    if (sidebarOpen()) persistLeftOpen(false);
  }
}

export function dismissMobileDrawer(_reason: "explicit" | "scrim" | "escape" | "back" | "navigation"): boolean {
  const active = activeDrawer();
  if (!active) return false;
  if (active === "right") setRightSidebarOpenRaw(false);
  else persistLeftOpen(false);
  return true;
}

/** The only completion boundary for ordinary in-window navigation originating
 * in the left sidebar.  Sidebar rows call this after their destination has
 * opened (and graph rows only after the awaited graph outcome proves success).
 * At regular widths there is no active drawer, so navigation retains the
 * persistent desktop sidebar and does not move focus. */
export function completeActiveLeftNavigation(): boolean {
  if (activeDrawer() !== "left") return false;
  if (!dismissMobileDrawer("navigation")) return false;
  restoreDrawerFocus("navigation");
  return true;
}
export function setRightSidebarItemCollapsed(idx: number, collapsed: boolean) {
  setRightSidebar(rightSidebar().map((item, i) => i === idx ? { ...item, collapsed } : item));
}
export function setAllRightSidebarItemsCollapsed(collapsed: boolean) {
  setRightSidebar(rightSidebar().map((item) => ({ ...item, collapsed })));
}
export function closeRightSidebarItem(idx: number) {
  setRightSidebar(rightSidebar().filter((_, i) => i !== idx));
}
export function closeAllRightSidebarItems() {
  setRightSidebar([]);
}
/** Move a right-sidebar item to a new position (GH #211 drag-reorder). Order
 *  persists through the same setRightSidebar owner (localStorage + session). */
export function moveRightSidebarItem(from: number, to: number) {
  const items = rightSidebar();
  if (from === to || from < 0 || to < 0 || from >= items.length || to >= items.length) return;
  const next = [...items];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  setRightSidebar(next);
}

/** Remove block items whose live targets were structurally deleted. Their
 * collapse preference lives on the item, so no parallel stale-state map can
 * survive the removal. */
export function removeDeletedBlocksFromSidebar(uuids: ReadonlySet<string>) {
  if (!uuids.size) return;
  const next = rightSidebar().filter((item) => item.kind !== "block" || !uuids.has(item.uuid));
  if (next.length !== rightSidebar().length) setRightSidebar(next);
}

/** Swap a restored positional sidebar item for its settled form (same slot). */
export function replaceSidebarBlock(from: SidebarBlock, to: SidebarBlock): void {
  const items = rightSidebar();
  if (!items.includes(from)) return;
  setRightSidebar(items.map((item) => (item === from ? to : item)));
}

/** Resolve block items in parallel; remove and save only confirmed missing targets.
 * Failed lookups remain and toast; pages are untouched. O(block items) backend calls plus save. */
export async function pruneSidebarBlocks(): Promise<void> {
  const root = graphMeta()?.root;
  const owner = graphOwner(() => graphMeta()?.root === root);
  // A positional item names an ID-less block, which the backend cannot resolve by
  // uuid; the sidebar settles or drops it when its page loads.
  const blocks = rightSidebar().filter((i): i is SidebarBlock => i.kind === "block" && !i.blockPos);
  if (!blocks.length) return;
  const result = await readOwned(owner, Promise.allSettled(blocks.map((b) => backend().resolveBlock(b.uuid))));
  if (result.kind === "stale") return;
  const resolved = result.value;
  const dead = new Set(blocks.filter((_, i) =>
    resolved[i].status === "fulfilled" && !resolved[i].value));
  if (resolved.some((result) => result.status === "rejected")) {
    pushToast("Could not check some sidebar blocks. Try again after the graph loads.", "error");
  }
  if (dead.size) {
    setRightSidebar(rightSidebar().filter((i) => i.kind !== "block" || !dead.has(i)));
  }
}

// Right-click context menu — universal over its target (a block or a page),
// mirroring the sidebar's two reference kinds.
/** Structural-removal context for a sheet cell's right-click menu. `rowId` is the
 *  row block to drop (works for grid + table, sort-safe). `gridId`/`col` enable a
 *  positional column delete (grid only; tables remove columns via "Remove from
 *  schema" on the header instead). */
export type SheetCellRemoveCtx = { rowId?: string; gridId?: string; col?: number };

export type CtxTarget =
  | { kind: "block"; blockId: string }
  | {
      kind: "page";
      name: string;
      pageKind: "journal" | "page";
      path?: string;
      fileActions?: boolean;
      /** Exact title-row owner for focus restoration after menu dismissal. */
      focusOwner?: HTMLElement;
    }
  | { kind: "blockref"; uuid: string; page: string; pageKind: "journal" | "page"; path?: string }
  | { kind: "sheet-cell"; blockId: string; remove?: SheetCellRemoveCtx }
  | {
      kind: "sheet";
      ownerId: string;
      surface: "grid" | "table" | "board";
      rowSource: "children" | "query";
      groupBy?: string | null;
      schemaPage?: string;
      fields?: readonly string[];
      formulas?: readonly [string, string][];
      filter?: string | null;
    }
  | { kind: "action-menu"; items: readonly ContextMenuAction[] };
export interface ContextMenuAction {
  label: string;
  run?: () => void;
  disabled?: boolean;
  danger?: boolean;
  children?: readonly ContextMenuAction[];
}
export const [contextMenu, setContextMenu] = graphScopedSignal<
  { x: number; y: number } & CtxTarget
>();
export function openContextMenu(x: number, y: number, blockId: string) {
  setContextMenu({ x, y, kind: "block", blockId });
}
export function openPageContextMenu(
  x: number,
  y: number,
  target: PageTarget,
  fileActions?: boolean,
  focusOwner?: HTMLElement,
): void;
export function openPageContextMenu(
  x: number,
  y: number,
  name: string,
  pageKind?: "journal" | "page",
  fileActions?: boolean,
  focusOwner?: HTMLElement,
): void;
export function openPageContextMenu(
  x: number,
  y: number,
  targetOrName: PageTarget | string,
  pageKindOrFileActions: PageKind | boolean = "page",
  fileActionsOrFocus?: boolean | HTMLElement,
  focusOwner?: HTMLElement,
) {
  const target: PageTarget = typeof targetOrName === "string"
    ? { name: targetOrName, pageKind: pageKindOrFileActions as PageKind }
    : targetOrName;
  const fileActions = typeof targetOrName === "string"
    ? (typeof fileActionsOrFocus === "boolean" ? fileActionsOrFocus : false)
    : (typeof pageKindOrFileActions === "boolean" ? pageKindOrFileActions : false);
  const owner = typeof targetOrName === "string"
    ? focusOwner
    : (fileActionsOrFocus instanceof HTMLElement ? fileActionsOrFocus : undefined);
  setContextMenu({ x, y, kind: "page", ...target, fileActions, focusOwner: owner });
}
export function openBlockRefContextMenu(
  x: number,
  y: number,
  uuid: string,
  page: string,
  pageKind: "journal" | "page" = "page",
  path?: string,
) {
  setContextMenu({ x, y, kind: "blockref", uuid, page, pageKind, path });
}
export function openSheetCellContextMenu(x: number, y: number, blockId: string, remove?: SheetCellRemoveCtx) {
  setContextMenu({ x, y, kind: "sheet-cell", blockId, remove });
}
export function openSheetContextMenu(
  x: number,
  y: number,
  ownerId: string,
  surface: "grid" | "table" | "board",
  rowSource: "children" | "query",
  groupBy?: string | null,
  opts: {
    schemaPage?: string;
    fields?: readonly string[];
    formulas?: readonly [string, string][];
    filter?: string | null;
  } = {}
) {
  setContextMenu({ x, y, kind: "sheet", ownerId, surface, rowSource, groupBy, ...opts });
}
export function openActionContextMenu(x: number, y: number, items: readonly ContextMenuAction[]) {
  setContextMenu({ x, y, kind: "action-menu", items });
}
export function closeContextMenu() {
  setContextMenu(null);
}

// A navigation surface can request that the ordinary per-block referrer panel
// open when its target block mounts. The monotonically increasing token makes a
// repeated request for the same block observable after the user closed it.
let blockReferencesRequestToken = 0;
export const [blockReferencesRequest, setBlockReferencesRequest] = graphScopedSignal<{
  id: string;
  token: number;
}>();
export function requestBlockReferences(id: string) {
  setBlockReferencesRequest({ id, token: ++blockReferencesRequestToken });
}

export type SettingsTabId = "appearance" | "editor" | "journals" | "files" | "backups" | "graph" | "plugins" | "diagnostics" | "shortcuts" | "about";

export const [settingsOpen, setSettingsOpen] = createSignal(false);

// A graph switch/restore/close is a quiescent state transition: once true, the
// current editor has been committed and no UI interaction may create another
// mutation until the transition either installs the new graph or aborts.
export const [graphTransitioning, setGraphTransitioning] = createSignal(false);
export const [settingsTabRequest, setSettingsTabRequest] = createSignal<SettingsTabId | null>(null);
export function openSettings(tab?: SettingsTabId) {
  if (tab) setSettingsTabRequest(tab);
  setSettingsOpen(true);
}
export function closeSettings() {
  setSettingsOpen(false);
}
export function clearSettingsTabRequest() {
  setSettingsTabRequest(null);
}

export const [helpPopupOpen, setHelpPopupOpen] = createSignal(false);
export function toggleHelpPopup() {
  setHelpPopupOpen((open) => !open);
}
export function closeHelpPopup() {
  setHelpPopupOpen(false);
}

export const [welcomeOpen, setWelcomeOpen] = createSignal(false);
export function openWelcome() {
  setWelcomeOpen(true);
}
export function closeWelcome() {
  setWelcomeOpen(false);
}

// Full-screen image lightbox (click an inline image to zoom).
export const [lightbox, setLightbox] = createSignal<string | null>(null);
// The images the viewer can page through (GH #501), starting at the opened one,
// and which of them is showing. `lightbox()` stays the single source of truth for
// the displayed src; when it does not match the gallery slot (a plain
// setLightbox(src) caller) the viewer shows just that image.
export const [lightboxGallery, setLightboxGallery] = createSignal<string[]>([]);
export const [lightboxIndex, setLightboxIndex] = createSignal(0);
export function openLightbox(src: string, gallery?: string[]) {
  const list = gallery && gallery.length > 0 ? gallery : [src];
  setLightboxGallery(list);
  setLightboxIndex(0);
  setLightbox(src);
}

// Expanded audio player overlay (the "Expand" button on an inline audio embed):
// a dimmed, ~90%-wide panel with a waveform scrubber + skip controls. `url` is the
// markdown asset URL (resolved to a blob like the inline embed); `name` is the
// display filename. Null = closed.
export const [audioPlayer, setAudioPlayer] =
  createSignal<{ url: string; name: string } | null>(null);

export { pageIdentityKey };

export const [switcherOpen, setSwitcherOpen] = createSignal(false);
export const [switcherPluginBlock, setSwitcherPluginBlock] = createSignal<OwnedPluginBlockSnapshot | null>(null);
// "all" = full Ctrl-K (pages/create/commands/blocks); "commands" = command
// palette (⌘⇧P), commands only.
export type SwitcherMode = "all" | "commands" | "current-page";
export const [switcherMode, setSwitcherMode] = createSignal<SwitcherMode>("all");
export const [switcherEmbryo, setSwitcherEmbryo] =
  createSignal<{ paneId: string; prefill: string } | null>(null);
export function openSwitcher(opts?: { mode?: "embryo" | "current-page"; paneId?: string; prefill?: string; pluginBlock?: OwnedPluginBlockSnapshot | null }) {
  setSwitcherMode(opts?.mode === "current-page" ? "current-page" : "all");
  setSwitcherPluginBlock(opts?.pluginBlock ?? null);
  setSwitcherEmbryo(opts?.mode === "embryo" && opts.paneId
    ? { paneId: opts.paneId, prefill: opts.prefill ?? "" }
    : null);
  setSwitcherOpen(true);
}
/** Toggle the WebView developer tools (WebKit Web Inspector) for theme/CSS
 *  debugging (GH #31). No-op in the mock; on a release build it works because the
 *  `devtools` Cargo feature is enabled. */
export function openDevtools() {
  void backend().openDevtools();
}
export function openCommandPalette(pluginBlock: OwnedPluginBlockSnapshot | null = null) {
  setSwitcherMode("commands");
  setSwitcherEmbryo(null);
  setSwitcherPluginBlock(pluginBlock);
  setSwitcherOpen(true);
}
export function closeSwitcher() {
  setSwitcherOpen(false);
  setSwitcherEmbryo(null);
  setSwitcherPluginBlock(null);
}

// PDF export: the page whose export-options dialog is open (null = closed). Set by
// the page context menu / the "Export current page to PDF" command; the dialog
// collects options and calls exportPagePdf.
export const [pdfExportPage, setPdfExportPage] = graphScopedSignal<string>();
export function openPdfExport(name: string) {
  if (isMobilePlatform) {
    pushToast("PDF export needs the desktop app: a mobile WebView cannot print.", "info");
    return;
  }
  setPdfExportPage(name);
}
export function closePdfExport() {
  setPdfExportPage(null);
}

/** Effective graph-local OG accent-removal setting for frontend search views. */
export function searchRemoveAccents(): boolean {
  return graphMeta()?.enable_search_remove_accents !== false;
}

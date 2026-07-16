// Small global UI state: theme, left sidebar, and the quick-switcher modal.
import { createSignal, useContext } from "solid-js";
import type { GraphMeta, JournalConflict, SyncConflict, PageKind } from "./types";
import { backend, isTauri } from "./backend";
// Zoom is route state; these are call-time only, so the ui↔router cycle is safe.
import { route, focusBlock, scheduleSessionSave } from "./router";
import { PaneContext } from "./paneContext";
import { exitPaneSelect } from "./paneSelect";
import { setJournalTitleFormat, isJournalTitle } from "./journal";

const THEME_KEY = "logseq-claude.theme";
function loadTheme(): "light" | "dark" {
  try {
    const t = localStorage.getItem(THEME_KEY);
    if (t === "dark" || t === "light") return t;
  } catch {
    // ignore
  }
  return "light";
}
export const [theme, setTheme] = createSignal<"light" | "dark">(loadTheme());

/** Apply the stored theme to the document (call once at startup). */
export function applyTheme() {
  document.documentElement.setAttribute("data-theme", theme());
  void backend().setSystemBarAppearance(theme() === "dark").catch(() => {});
}

// Task workflow from config.edn (:preferred-workflow): drives mod+enter cycling.
export const [workflow, setWorkflow] = createSignal<"now" | "todo">("now");
/** Set the workflow and persist it to config.edn (graph-portable, like Logseq).
 *  The signal is the runtime source of truth; the file is re-read on next open. */
export function changeWorkflow(wf: "now" | "todo") {
  if (wf === workflow()) return;
  setWorkflow(wf);
  void backend().setPreferredWorkflow(wf).catch(() => {});
}

export function timetrackingEnabled(): boolean {
  return graphMeta()?.enable_timetracking ?? true;
}

export function logbookWithSecondSupport(): boolean {
  return graphMeta()?.logbook_with_second_support ?? true;
}

export function changeTimetrackingEnabled(enabled: boolean) {
  const m = graphMeta();
  if (m && m.enable_timetracking === enabled) return;
  if (m) setGraphMeta({ ...m, enable_timetracking: enabled });
  void backend().setTimetrackingEnabled(enabled).catch(() => {});
}

// --- appearance: accent color, wide mode, document mode (all persisted) ---
function loadStr(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function saveStr(key: string, val: string | null) {
  try {
    if (val === null) localStorage.removeItem(key);
    else localStorage.setItem(key, val);
  } catch {
    // ignore
  }
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
  setAccentColor(c);
  saveStr(ACCENT_KEY, c);
  applyAccent();
}

const WIDE_KEY = "logseq-claude.wide";
const DOC_KEY = "logseq-claude.doc-mode";
export const [wideMode, setWideMode] = createSignal(loadStr(WIDE_KEY) === "1");
export const [documentMode, setDocumentMode] = createSignal(loadStr(DOC_KEY) === "1");
export function toggleWideMode() {
  const v = !wideMode();
  setWideMode(v);
  saveStr(WIDE_KEY, v ? "1" : null);
}
export function toggleDocumentMode() {
  const v = !documentMode();
  setDocumentMode(v);
  saveStr(DOC_KEY, v ? "1" : null);
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
  setTypographyModeSig(m);
  // Persist only the non-default; absent key ⇒ "render".
  saveStr(TYPO_KEY, m === "render" ? null : m);
  bumpGraphEpoch(); // re-render open pages so the change is immediate
}

// --- editor auto-pairing (a Tine convenience, OFF by default): typing `(`/`[`/
// `{`/`"`/backtick inserts the matching closer (caret between), wraps a selection,
// types-through a closer, and Backspace deletes an empty pair. The always-on
// OG-style `[[`→`[[]]` page-ref pairing is separate (autoPairEdit) and unaffected.
// Local editor pref, persisted like the others. ---
const AUTOPAIR_KEY = "logseq-claude.autopair";
export const [autoPairing, setAutoPairingSig] = createSignal(loadStr(AUTOPAIR_KEY) === "1");
export function setAutoPairing(v: boolean) {
  setAutoPairingSig(v);
  saveStr(AUTOPAIR_KEY, v ? "1" : null);
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
 *  immediately. */
export function changeStartOfWeek(l: number) {
  const n = Math.min(6, Math.max(0, Math.floor(l)));
  const m = graphMeta();
  if (m) setGraphMeta({ ...m, start_of_week: n });
  void backend().setStartOfWeek(n).catch(() => {});
}

/** Persist the format new pages/journals are created in (`:preferred-format`)
 *  and update graphMeta optimistically. Existing files keep their own format. */
export function changePreferredFormat(fmt: "md" | "org") {
  const m = graphMeta();
  if (!m || m.preferred_format === fmt) return;
  setGraphMeta({ ...m, preferred_format: fmt });
  void backend().setPreferredFormat(fmt).catch(() => {});
}

/** Change the journal display-title format (`:journal/page-title-format`).
 *  Optimistically updates the in-memory formatter + meta and bumps the graph
 *  epoch so open journal titles re-render; persists to config.edn. Display-only
 *  — journal file names (`:journal/file-name-format`) are unaffected. */
export function changeJournalTitleFormat(fmt: string) {
  const next = fmt.trim() || "MMM do, yyyy";
  const m = graphMeta();
  if (!m || m.journal_page_title_format === next) return;
  setGraphMeta({ ...m, journal_page_title_format: next });
  setJournalTitleFormat(next);
  bumpGraphEpoch(); // immediate: re-render open journal titles with the new format
  // The backend rewrites config.edn AND reopens the graph (so its journal_format
  // + the title-named-journal migration take effect). Bump again once that's done
  // so the feed reloads against the refreshed backend — otherwise a reload racing
  // the reopen could re-query the old format.
  void backend()
    .setJournalTitleFormat(next)
    .then(() => {
      bumpGraphEpoch();
      void refreshJournalConflicts(true); // surface any days the migration couldn't merge
    })
    .catch(() => {});
}

// --- duplicate journal days (a date with >1 file, e.g. a date-stem file + a
// title-named one). The filename migration never clobbers, so these are left for
// the user to reconcile; we surface them rather than letting a day silently show
// twice in the feed. ---
export const [journalConflicts, setJournalConflicts] = createSignal<JournalConflict[]>([]);
/** Re-fetch the duplicate-journal-day list; with `notify`, toast if any exist. */
export async function refreshJournalConflicts(notify = false): Promise<void> {
  try {
    const c = await backend().listJournalConflicts();
    setJournalConflicts(c);
    if (notify && c.length) {
      pushToast(
        `${c.length} journal day${c.length === 1 ? "" : "s"} have duplicate files in different formats — reconcile them in Settings → Backups & recovery`,
        "info",
        { sticky: true, action: { label: "Open", run: () => openSettings("backups") } }
      );
    }
  } catch {
    /* best-effort */
  }
}

// --- sync-tool conflict copies (Syncthing/Dropbox `*.sync-conflict-*` files).
// Excluded from the page list; surfaced here so the user can review + merge them
// (Settings → Backups & recovery) instead of them rotting as garbage pages. ---
export const [syncConflicts, setSyncConflicts] = createSignal<SyncConflict[]>([]);
/** Re-fetch the sync-conflict list; with `notify`, toast if any exist. */
export async function refreshSyncConflicts(notify = false): Promise<void> {
  try {
    const c = await backend().listSyncConflicts();
    setSyncConflicts(c);
    if (notify && c.length) {
      pushToast(
        `${c.length} sync-conflict file${c.length === 1 ? "" : "s"} in your graph — review + merge them in Settings → Backups & recovery`,
        "info",
        { sticky: true, action: { label: "Open", run: () => openSettings("backups") } }
      );
    }
  } catch {
    /* best-effort */
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
 *  "notes" for zoom purposes. Returns an uninstaller. */
export function installPaneTracker(): () => void {
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

// When a query block is created via the "/Query (visual builder)" command, hold
// its block id so the freshly-rendered QueryBuilder opens its add-filter picker
// immediately (the block id, consumed once on mount, then cleared).
export const [queryBuilderAutoOpen, setQueryBuilderAutoOpen] = createSignal<string | null>(null);

// Page-properties panel (alias / public / tags / icon / title), opened from the
// page-title gear or the "/Page properties" command. Anchored at x,y.
export const [pagePropsPanel, setPagePropsPanel] = createSignal<{ name: string; x: number; y: number } | null>(null);
export function openPageProps(name: string, x: number, y: number) {
  setPagePropsPanel({ name, x, y });
}
export function closePageProps() {
  setPagePropsPanel(null);
}

// "Copy / export as" modal — a live-preview text export of a block subtree or a
// multi-block selection, with indent-style + remove options (mirrors OG Logseq).
export const [exportModal, setExportModal] = createSignal<{ ids: string[] } | null>(null);
export function openExportModal(ids: string[]) {
  if (ids.length) setExportModal({ ids });
}
export function closeExportModal() {
  setExportModal(null);
}

// Remember the window's pre-focus fullscreen state so exiting focus restores it
// (rather than always dropping out of fullscreen if the user was already in it).
let preFocusFullscreen = false;
async function appWindow() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}
export function toggleFocusMode() {
  if (focusMode()) void exitFocusMode();
  else void enterFocusMode();
}
export async function enterFocusMode() {
  if (focusMode()) return;
  // When the setting is on, focus mode owns dim: on while focused, off when
  // exited (a transient signal change — it doesn't rewrite the t-b preference).
  if (dimInFocus()) setDimInactiveBlocks(true);
  setFocusMode(true);
  if (!isTauri()) return;
  try {
    const w = await appWindow();
    preFocusFullscreen = await w.isFullscreen();
    if (!preFocusFullscreen) await w.setFullscreen(true);
  } catch {
    // ignore (window plugin unavailable)
  }
}
export async function exitFocusMode() {
  if (!focusMode()) return;
  setFocusMode(false);
  if (dimInFocus()) setDimInactiveBlocks(false);
  if (!isTauri()) return;
  try {
    if (!preFocusFullscreen) (await appWindow()).setFullscreen(false);
  } catch {
    // ignore
  }
}

// Loaded graph metadata (root path, dirs, shortcut overrides), for Settings.
export const [graphMeta, setGraphMeta] = createSignal<GraphMeta | null>(null);

// True once the startup graph-load attempt has finished (success OR failure). The
// onboarding Welcome screen shows only when this is set AND no graph loaded — so a
// fresh install with no configured graph gets the wizard, but a normal startup
// never flashes it while the graph is still loading.
export const [firstLoadDone, setFirstLoadDone] = createSignal(false);

/** Set (or clear, with null) the template applied to new journal days, persisting
 *  it to config.edn `:default-templates {:journals "Name"}` and updating the live
 *  meta so the UI reflects it immediately. */
export function setJournalTemplate(name: string | null) {
  const m = graphMeta();
  const prev = m?.default_journal_template ?? null;
  if (m) setGraphMeta({ ...m, default_journal_template: name });
  // On a config-write failure, revert the optimistic UI + tell the user, rather
  // than silently showing a template that wasn't actually persisted.
  void backend()
    .setDefaultJournalTemplate(name)
    .catch((e) => {
      const cur = graphMeta();
      if (cur) setGraphMeta({ ...cur, default_journal_template: prev });
      pushToast(`Couldn't save the journal template setting. (${String(e)})`, "error");
    });
}
// Bumped when the open graph changes, so views reload against the new graph.
export const [graphEpoch, setGraphEpoch] = createSignal(0);
export function bumpGraphEpoch() {
  setGraphEpoch((n) => n + 1);
}

// Bumped after a save batch lands (the Rust cache now reflects the edit), so
// derived whole-graph views — {{query}} results, backlinks — can recompute.
// This is Tine's stand-in for OG's reactive-DB query invalidation.
export const [dataRev, setDataRev] = createSignal(0);
export function bumpDataRev() {
  setDataRev((n) => n + 1);
}
export function toggleTheme() {
  const next = theme() === "light" ? "dark" : "light";
  setTheme(next);
  document.documentElement.setAttribute("data-theme", next);
  void backend().setSystemBarAppearance(next === "dark").catch(() => {});
  try {
    localStorage.setItem(THEME_KEY, next);
  } catch {
    // ignore
  }
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

export function toggleSidebar() {
  const v = !sidebarOpen();
  setSidebarOpen(v);
  saveStr(SIDEBAR_OPEN_KEY, v ? null : "0");
  scheduleSessionSave(); // durable open/closed state (localStorage isn't kept)
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
  if (Array.isArray(s.items)) setRightSidebarRaw(s.items.filter(validSidebarItem));
  if (typeof s.right === "boolean") setRightSidebarOpenSig(s.right);
  setFavoritesSectionExpanded(s.favoritesExpanded ?? true);
  setRecentSectionExpanded(s.recentExpanded ?? true);
}

const SIDEBAR_W_KEY = "logseq-claude.sidebarWidth";
function loadSidebarWidth(): number {
  try {
    const v = Number(localStorage.getItem(SIDEBAR_W_KEY));
    if (v >= 180 && v <= 600) return v;
  } catch {
    // ignore
  }
  return 246;
}
export const [sidebarWidth, setSidebarWidth] = createSignal(loadSidebarWidth());
export function persistSidebarWidth() {
  try {
    localStorage.setItem(SIDEBAR_W_KEY, String(sidebarWidth()));
  } catch {
    // ignore
  }
}

const RS_W_KEY = "logseq-claude.rightSidebarWidth";
function loadRsWidth(): number {
  try {
    const v = Number(localStorage.getItem(RS_W_KEY));
    if (v >= 220 && v <= 800) return v;
  } catch {
    // ignore
  }
  return 360;
}
export const [rightSidebarWidth, setRightSidebarWidth] = createSignal(loadRsWidth());
export function persistRightSidebarWidth() {
  try {
    localStorage.setItem(RS_W_KEY, String(rightSidebarWidth()));
  } catch {
    // ignore
  }
}

const PDF_W_KEY = "logseq-claude.pdfPaneWidth";
function loadPdfWidth(): number {
  try {
    const v = Number(localStorage.getItem(PDF_W_KEY));
    if (v >= 320 && v <= 1200) return v;
  } catch {
    // ignore
  }
  return 560;
}
export const [pdfPaneWidth, setPdfPaneWidth] = createSignal(loadPdfWidth());
export function persistPdfPaneWidth() {
  try {
    localStorage.setItem(PDF_W_KEY, String(pdfPaneWidth()));
  } catch {
    // ignore
  }
}

// Favorites (starred pages/journals). Persisted PER GRAPH in that graph's
// config.edn `:favorites` (the single source of truth) — NOT in a global
// localStorage key, which would leak one graph's favorites into another and
// leave dead links (clicking them opens an empty page) after a graph switch.
// The signal starts empty and is (re)seeded from config.edn on every graph open
// (see seedFavorites), so switching graphs always shows exactly that graph's set.
export interface FavItem {
  name: string;
  kind: PageKind;
}
export const [favorites, setFavorites] = createSignal<FavItem[]>([]);
export function isFavorite(name: string): boolean {
  const target = resolveAlias(name);
  return favorites().some((f) =>
    f.kind === "page" ? resolveAlias(f.name) === target : f.name === name
  );
}
function persistFavorites(next: FavItem[]) {
  // Persist to config.edn :favorites so favorites travel with the graph and stay
  // scoped to it. config.edn stores names only; kind is re-derived on seed.
  void backend().setFavorites(next.map((f) => f.name)).catch(() => {});
}
export function toggleFavorite(name: string, kind: "page" | "journal" = "page") {
  const f = favorites();
  const target = kind === "page" ? resolveAlias(name) : name;
  const matches = (item: FavItem) => item.kind === kind &&
    (kind === "page" ? resolveAlias(item.name) === target : item.name === name);
  const next = f.some(matches)
    ? f.filter((x) => !matches(x))
    : [...f, { name, kind }];
  setFavorites(next);
  persistFavorites(next);
}
export function removeDeletedPageFromNavigation(name: string, kind: PageKind) {
  const nextFavs = favorites().filter((f) => f.name !== name);
  if (nextFavs.length !== favorites().length) {
    setFavorites(nextFavs);
    persistFavorites(nextFavs);
  }

  const nextRecents = recentPages().filter((r) => !(r.name === name && r.kind === kind));
  if (nextRecents.length !== recentPages().length) {
    setRecentPages(nextRecents);
    try {
      if (nextRecents.length) localStorage.setItem(RECENT_KEY, JSON.stringify(nextRecents));
      else localStorage.removeItem(RECENT_KEY);
    } catch {
      // ignore
    }
  }

  const nextSidebar = rightSidebar().filter((item) =>
    item.kind === "page" ? item.name !== name : item.page !== name
  );
  if (nextSidebar.length !== rightSidebar().length) setRightSidebar(nextSidebar);
}

/** Re-key sidebar navigation state after the backend has atomically renamed a
 * page. Keep ordering stable, collapse an existing destination duplicate, and
 * persist both stores before the subsequent openPage(next) promotes the one
 * canonical recent entry to the front. */
export function renamePageInNavigation(from: string, to: string) {
  const dedupe = (items: FavItem[]): FavItem[] => {
    const seen = new Set<string>();
    const out: FavItem[] = [];
    for (const item of items) {
      const next = item.kind === "page" && item.name === from ? { ...item, name: to } : item;
      const key = `${next.kind}\0${next.name}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(next);
    }
    return out;
  };

  const nextFavorites = dedupe(favorites());
  if (nextFavorites.some((item, i) => item !== favorites()[i]) || nextFavorites.length !== favorites().length) {
    setFavorites(nextFavorites);
    persistFavorites(nextFavorites);
  }

  const nextRecents = dedupe(recentPages());
  if (nextRecents.some((item, i) => item !== recentPages()[i]) || nextRecents.length !== recentPages().length) {
    setRecentPages(nextRecents);
    try {
      if (nextRecents.length) localStorage.setItem(RECENT_KEY, JSON.stringify(nextRecents));
      else localStorage.removeItem(RECENT_KEY);
    } catch {
      // ignore
    }
  }

  const seenSidebar = new Set<string>();
  const nextSidebar: SidebarItem[] = [];
  for (const item of rightSidebar()) {
    const next: SidebarItem = item.kind === "page"
      ? (item.name === from ? { ...item, name: to } : item)
      : (item.page === from ? { ...item, page: to } : item);
    const key = sidebarItemKey(next);
    if (seenSidebar.has(key)) continue;
    seenSidebar.add(key);
    nextSidebar.push(next);
  }
  if (nextSidebar.some((item, i) => item !== rightSidebar()[i]) || nextSidebar.length !== rightSidebar().length) {
    setRightSidebar(nextSidebar);
  }
}
/** Seed favorites from config.edn `:favorites` on graph open. config.edn is the
 *  source of truth, so this ALWAYS replaces the current set — including clearing
 *  it to empty when the newly-opened graph has no favorites — otherwise the
 *  previous graph's favorites would linger and open empty pages. config.edn
 *  stores names only; kind is re-derived so a favorited journal still routes as a
 *  journal (not a would-be-empty page). */
export function seedFavorites(names: string[]) {
  setFavorites(
    names.map((name): FavItem => ({ name, kind: isJournalTitle(name) ? "journal" : "page" }))
  );
}

// Recently-visited pages (navigation history), newest first. This is the right
// signal for the sidebar's "recent" list — file mtime is unreliable (a restored
// backup makes every file look freshly modified). Persisted locally.
const RECENT_KEY = "logseq-claude.recent";
function loadRecent(): FavItem[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}
export const [recentPages, setRecentPages] = createSignal<FavItem[]>(loadRecent());
/** Drop the recent-pages list. Called on a graph SWITCH so the previous graph's
 *  pages don't linger in quick-switch (Ctrl-K) / the sidebar "recent" list. */
export function clearRecent() {
  setRecentPages([]);
  try {
    localStorage.removeItem(RECENT_KEY);
  } catch {
    // ignore
  }
}
export function pushRecent(name: string, kind: "page" | "journal" = "page") {
  const cur = recentPages().filter((r) => !(r.name === name && r.kind === kind));
  const next = [{ name, kind }, ...cur].slice(0, 20);
  setRecentPages(next);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // ignore
  }
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
  setShortcutOverrides(next);
  try {
    localStorage.setItem(SHORTCUTS_KEY, JSON.stringify(next));
  } catch {
    // ignore
  }
}
export function setShortcutOverride(id: string, binding: string) {
  persistShortcuts({ ...shortcutOverrides(), [id]: binding });
}
export function resetShortcutOverride(id: string) {
  const next = { ...shortcutOverrides() };
  delete next[id];
  persistShortcuts(next);
}

// Pages that failed to save because the file changed on disk (external edit /
// Syncthing). Surfaced as a banner; the user resolves with reload or overwrite.
export const [conflicts, setConflicts] = createSignal<string[]>([]);
export function markConflict(name: string) {
  if (!conflicts().includes(name)) setConflicts([...conflicts(), name]);
}
export function clearConflict(name: string) {
  setConflicts(conflicts().filter((n) => n !== name));
}
export function isConflicted(name: string): boolean {
  return conflicts().includes(name);
}

// Date picker popup for SCHEDULED / DEADLINE and typed sheet date properties.
export type DatePickerTarget =
  | "scheduled"
  | "deadline"
  | { field: `prop:${string}`; fieldType: "date" | "datetime" };
export const [datePicker, setDatePicker] = createSignal<
  { blockId: string; which: DatePickerTarget; x: number; y: number } | null
>(null);
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
export const [formulaEditor, setFormulaEditor] = createSignal<FormulaEditorTarget | null>(null);
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
  collapsed?: boolean;
}
export interface SidebarBlock {
  kind: "block";
  uuid: string; // stable block id — the live handle into the store
  page: string; // the page it lives on (loaded on demand)
  pageKind: "journal" | "page";
  collapsed?: boolean;
}
export type SidebarItem = SidebarPage | SidebarBlock;

/** Stable presentation identity for one sidebar collection item. Unlike an
 * array index, it survives closing a neighbor and page renames are re-keyed by
 * renamePageInNavigation. */
export function sidebarItemKey(item: SidebarItem): string {
  return item.kind === "page"
    ? `page:${item.pageKind}:${item.name}`
    : `block:${item.uuid}`;
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
  if (o.kind === "page") return typeof o.name === "string";
  if (o.kind === "block") return typeof o.uuid === "string" && typeof o.page === "string";
  return false;
}
function loadRsItems(): SidebarItem[] {
  try {
    const raw = localStorage.getItem(RS_ITEMS_KEY);
    const arr = raw ? JSON.parse(raw) : [];
    return Array.isArray(arr) ? arr.filter(validSidebarItem) : [];
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
export const [rightSidebarOpen, setRightSidebarOpenSig] = createSignal(
  loadStr(RS_OPEN_KEY) === "1" || rightSidebar().length > 0
);
function setRightSidebarOpen(v: boolean) {
  setRightSidebarOpenSig(v);
  saveStr(RS_OPEN_KEY, v ? "1" : null);
  scheduleSessionSave(); // durable open/closed state (localStorage isn't kept)
}
export function toggleRightSidebar() {
  setRightSidebarOpen(!rightSidebarOpen());
}
export function setRightSidebar(items: SidebarItem[]) {
  setRightSidebarRaw(items);
  try {
    if (items.length) localStorage.setItem(RS_ITEMS_KEY, JSON.stringify(items));
    else localStorage.removeItem(RS_ITEMS_KEY);
  } catch {
    // ignore
  }
  scheduleSessionSave(); // durable right-sidebar items (localStorage isn't kept)
}

export function openPageInSidebar(name: string, pageKind: "journal" | "page" = "page") {
  if (pageKind === "page") name = resolveAlias(name);
  setRightSidebarOpen(true);
  const existing = rightSidebar().findIndex((i) => i.kind === "page" && i.name === name);
  if (existing >= 0) {
    setRightSidebar(rightSidebar().map((item, i) => i === existing ? { ...item, collapsed: false } : item));
    return;
  }
  setRightSidebar([{ kind: "page", name, pageKind }, ...rightSidebar()]);
}
export function openBlockInSidebar(ref: { uuid: string; page: string; pageKind: "journal" | "page" }) {
  setRightSidebarOpen(true);
  const existing = rightSidebar().findIndex((i) => i.kind === "block" && i.uuid === ref.uuid);
  if (existing >= 0) {
    setRightSidebar(rightSidebar().map((item, i) => i === existing ? { ...item, collapsed: false } : item));
    return;
  }
  setRightSidebar([{ kind: "block", ...ref }, ...rightSidebar()]);
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

/** Remove block items whose live targets were structurally deleted. Their
 * collapse preference lives on the item, so no parallel stale-state map can
 * survive the removal. */
export function removeDeletedBlocksFromSidebar(uuids: ReadonlySet<string>) {
  if (!uuids.size) return;
  const next = rightSidebar().filter((item) => item.kind !== "block" || !uuids.has(item.uuid));
  if (next.length !== rightSidebar().length) setRightSidebar(next);
}

/** Drop restored block items whose block can't be resolved (its in-memory uuid
 *  changed across the restart). Page items are left untouched. */
export async function pruneSidebarBlocks(): Promise<void> {
  const blocks = rightSidebar().filter((i): i is SidebarBlock => i.kind === "block");
  if (!blocks.length) return;
  const resolved = await Promise.all(
    blocks.map((b) => backend().resolveBlock(b.uuid).catch(() => null))
  );
  const dead = new Set(blocks.filter((_, i) => !resolved[i]).map((b) => b.uuid));
  if (dead.size) {
    setRightSidebar(rightSidebar().filter((i) => i.kind !== "block" || !dead.has(i.uuid)));
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
  | { kind: "page"; name: string; pageKind: "journal" | "page"; fileActions?: boolean }
  | { kind: "blockref"; uuid: string; page: string; pageKind: "journal" | "page" }
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
export const [contextMenu, setContextMenu] = createSignal<
  ({ x: number; y: number } & CtxTarget) | null
>(null);
export function openContextMenu(x: number, y: number, blockId: string) {
  setContextMenu({ x, y, kind: "block", blockId });
}
export function openPageContextMenu(
  x: number,
  y: number,
  name: string,
  pageKind: "journal" | "page" = "page",
  fileActions = false,
) {
  setContextMenu({ x, y, kind: "page", name, pageKind, fileActions });
}
export function openBlockRefContextMenu(
  x: number,
  y: number,
  uuid: string,
  page: string,
  pageKind: "journal" | "page" = "page"
) {
  setContextMenu({ x, y, kind: "blockref", uuid, page, pageKind });
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
export const [blockReferencesRequest, setBlockReferencesRequest] = createSignal<{
  id: string;
  token: number;
} | null>(null);
export function requestBlockReferences(id: string) {
  setBlockReferencesRequest({ id, token: ++blockReferencesRequestToken });
}

export type SettingsTabId = "appearance" | "editor" | "journals" | "files" | "backups" | "graph" | "improve" | "shortcuts" | "about";

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

// Transient toast notifications (bottom-right), auto-dismissed.
export interface Toast {
  id: number;
  message: string;
  kind: "info" | "success" | "warn" | "error";
  sticky?: boolean; // stays until the user closes it (✕); no auto-dismiss
  // Optional action button (e.g. "Download"). Runs, then dismisses the toast.
  action?: { label: string; run: () => void };
  onDismiss?: () => void;
}
let toastSeq = 0;
export const [toasts, setToasts] = createSignal<Toast[]>([]);
export function pushToast(
  message: string,
  kind: Toast["kind"] = "info",
  opts: { sticky?: boolean; action?: { label: string; run: () => void }; onDismiss?: () => void } = {}
): number {
  const id = ++toastSeq;
  setToasts([...toasts(), { id, message, kind, sticky: opts.sticky, action: opts.action, onDismiss: opts.onDismiss }]);
  if (!opts.sticky) setTimeout(() => dismissToast(id), 3200);
  return id;
}
export function dismissToast(id: number) {
  const toast = toasts().find((t) => t.id === id);
  toast?.onDismiss?.();
  setToasts(toasts().filter((t) => t.id !== id));
}

// Full-screen image lightbox (click an inline image to zoom).
export const [lightbox, setLightbox] = createSignal<string | null>(null);

// Expanded audio player overlay (the "Expand" button on an inline audio embed):
// a dimmed, ~90%-wide panel with a waveform scrubber + skip controls. `url` is the
// markdown asset URL (resolved to a blob like the inline embed); `name` is the
// display filename. Null = closed.
export const [audioPlayer, setAudioPlayer] =
  createSignal<{ url: string; name: string } | null>(null);

// Page aliases (alias:: → canonical), keyed by normalized alias; loaded per graph.
export const [aliasMap, setAliasMap] = createSignal<Record<string, string>>({});
/** Resolve a page name through `alias::` to its canonical page (else unchanged). */
export function resolveAlias(name: string): string {
  return aliasMap()[name.trim().toLowerCase()] ?? name;
}

export const [switcherOpen, setSwitcherOpen] = createSignal(false);
// "all" = full Ctrl-K (pages/create/commands/blocks); "commands" = command
// palette (⌘⇧P), commands only.
export type SwitcherMode = "all" | "commands";
export const [switcherMode, setSwitcherMode] = createSignal<SwitcherMode>("all");
export const [switcherEmbryo, setSwitcherEmbryo] =
  createSignal<{ paneId: string; prefill: string } | null>(null);
export function openSwitcher(opts?: { mode?: "embryo"; paneId?: string; prefill?: string }) {
  setSwitcherMode("all");
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
export function openCommandPalette() {
  setSwitcherMode("commands");
  setSwitcherEmbryo(null);
  setSwitcherOpen(true);
}
export function closeSwitcher() {
  setSwitcherOpen(false);
  setSwitcherEmbryo(null);
}

// PDF export: the page whose export-options dialog is open (null = closed). Set by
// the page context menu / the "Export current page to PDF" command; the dialog
// collects options and calls exportPagePdf.
export const [pdfExportPage, setPdfExportPage] = createSignal<string | null>(null);
export function openPdfExport(name: string) {
  setPdfExportPage(name);
}
export function closePdfExport() {
  setPdfExportPage(null);
}

// The PDF currently open in the side pane. `filename` is the stable resource
// identity; page/highlightId are a navigation intent within that resource.
// Keeping those concepts separate lets a second reference into the same PDF
// scroll precisely without tearing down the loaded document.
export interface PdfTarget {
  filename: string;
  label: string;
  page?: number;
  highlightId?: string;
}
export const [pdfTarget, setPdfTarget] = createSignal<PdfTarget | null>(null);
export function openPdf(filename: string, label: string, page?: number, highlightId?: string) {
  // Logseq treats re-opening the current PDF resource without a page/highlight
  // intent as a no-op. Preserve the reader's current location; explicit targets
  // within the same file still publish a new reactive navigation intent.
  if (pdfTarget()?.filename === filename && page == null && highlightId == null) return;
  setPdfTarget({ filename, label, page, highlightId });
}
export function closePdf() {
  setPdfTarget(null);
}

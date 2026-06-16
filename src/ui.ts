// Small global UI state: theme, left sidebar, and the quick-switcher modal.
import { createSignal } from "solid-js";
import type { GraphMeta } from "./types";

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
}

// Task workflow from config.edn (:preferred-workflow): drives mod+enter cycling.
export const [workflow, setWorkflow] = createSignal<"now" | "todo">("now");

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
/** Apply (or clear) the accent color as the link/highlight CSS variable. */
export function applyAccent() {
  const c = accentColor();
  const root = document.documentElement;
  if (c) root.style.setProperty("--link-color", c);
  else root.style.removeProperty("--link-color");
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

// Loaded graph metadata (root path, dirs, shortcut overrides), for Settings.
export const [graphMeta, setGraphMeta] = createSignal<GraphMeta | null>(null);
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
  try {
    localStorage.setItem(THEME_KEY, next);
  } catch {
    // ignore
  }
}

export const [sidebarOpen, setSidebarOpen] = createSignal(true);
export function toggleSidebar() {
  setSidebarOpen(!sidebarOpen());
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

// Bumped after a PDF highlight is written, so an open notes (hls__) page can
// reload itself to show the new highlight without a manual re-open.
export const [notesRefresh, setNotesRefresh] = createSignal<{ page: string; rev: number }>({
  page: "",
  rev: 0,
});
export function refreshNotes(page: string) {
  setNotesRefresh((prev) => ({ page, rev: prev.rev + 1 }));
}

// Favorites (starred pages/journals), persisted.
export interface FavItem {
  name: string;
  kind: "page" | "journal";
}
const FAV_KEY = "logseq-claude.favorites";
function loadFavs(): FavItem[] {
  try {
    const v = JSON.parse(localStorage.getItem(FAV_KEY) ?? "[]");
    if (!Array.isArray(v)) return [];
    // migrate old string[] format
    return v.map((x: unknown) =>
      typeof x === "string" ? { name: x, kind: "page" as const } : (x as FavItem)
    );
  } catch {
    return [];
  }
}
export const [favorites, setFavorites] = createSignal<FavItem[]>(loadFavs());
export function isFavorite(name: string): boolean {
  return favorites().some((f) => f.name === name);
}
export function toggleFavorite(name: string, kind: "page" | "journal" = "page") {
  const f = favorites();
  const next = f.some((x) => x.name === name)
    ? f.filter((x) => x.name !== name)
    : [...f, { name, kind }];
  setFavorites(next);
  try {
    localStorage.setItem(FAV_KEY, JSON.stringify(next));
  } catch {
    // ignore
  }
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

// Date picker popup for SCHEDULED / DEADLINE (and editing existing ones).
export const [datePicker, setDatePicker] = createSignal<
  { blockId: string; which: "scheduled" | "deadline"; x: number; y: number } | null
>(null);
export function openDatePicker(blockId: string, which: "scheduled" | "deadline", x: number, y: number) {
  setDatePicker({ blockId, which, x, y });
}
export function closeDatePicker() {
  setDatePicker(null);
}

// Block zoom: focus a single block's subtree (click its bullet). Session-level
// (the frontend block id stays valid while the page is loaded); cleared on nav.
export const [zoomedBlock, setZoomedBlock] = createSignal<string | null>(null);
export function zoomInto(id: string) {
  setZoomedBlock(id);
}
export function zoomOut() {
  setZoomedBlock(null);
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
}
export interface SidebarBlock {
  kind: "block";
  uuid: string; // stable block id — the live handle into the store
  page: string; // the page it lives on (loaded on demand)
  pageKind: "journal" | "page";
}
export type SidebarItem = SidebarPage | SidebarBlock;

export const [rightSidebar, setRightSidebar] = createSignal<SidebarItem[]>([]);

export function openPageInSidebar(name: string, pageKind: "journal" | "page" = "page") {
  if (rightSidebar().some((i) => i.kind === "page" && i.name === name)) return;
  setRightSidebar([{ kind: "page", name, pageKind }, ...rightSidebar()]);
}
export function openBlockInSidebar(ref: { uuid: string; page: string; pageKind: "journal" | "page" }) {
  if (rightSidebar().some((i) => i.kind === "block" && i.uuid === ref.uuid)) return;
  setRightSidebar([{ kind: "block", ...ref }, ...rightSidebar()]);
}
export function closeRightSidebarItem(idx: number) {
  setRightSidebar(rightSidebar().filter((_, i) => i !== idx));
}

// Right-click context menu — universal over its target (a block or a page),
// mirroring the sidebar's two reference kinds.
export type CtxTarget =
  | { kind: "block"; blockId: string }
  | { kind: "page"; name: string; pageKind: "journal" | "page" };
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
  pageKind: "journal" | "page" = "page"
) {
  setContextMenu({ x, y, kind: "page", name, pageKind });
}
export function closeContextMenu() {
  setContextMenu(null);
}

export const [settingsOpen, setSettingsOpen] = createSignal(false);
export function openSettings() {
  setSettingsOpen(true);
}
export function closeSettings() {
  setSettingsOpen(false);
}

export const [switcherOpen, setSwitcherOpen] = createSignal(false);
// "all" = full Ctrl-K (pages/create/commands/blocks); "commands" = command
// palette (⌘⇧P), commands only.
export type SwitcherMode = "all" | "commands";
export const [switcherMode, setSwitcherMode] = createSignal<SwitcherMode>("all");
export function openSwitcher() {
  setSwitcherMode("all");
  setSwitcherOpen(true);
}
export function openCommandPalette() {
  setSwitcherMode("commands");
  setSwitcherOpen(true);
}
export function closeSwitcher() {
  setSwitcherOpen(false);
}

// The PDF currently open in the side pane (filename within assets/, + label,
// + an optional page to scroll to).
export const [pdfTarget, setPdfTarget] = createSignal<{
  filename: string;
  label: string;
  page?: number;
} | null>(null);
export function openPdf(filename: string, label: string, page?: number) {
  setPdfTarget({ filename, label, page });
}
export function closePdf() {
  setPdfTarget(null);
}

// Small global UI state: theme, left sidebar, and the quick-switcher modal.
import { createSignal } from "solid-js";

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

// Favorites (starred pages), persisted.
const FAV_KEY = "logseq-claude.favorites";
function loadFavs(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(FAV_KEY) ?? "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}
export const [favorites, setFavorites] = createSignal<string[]>(loadFavs());
export function isFavorite(name: string): boolean {
  return favorites().includes(name);
}
export function toggleFavorite(name: string) {
  const f = favorites();
  const next = f.includes(name) ? f.filter((x) => x !== name) : [...f, name];
  setFavorites(next);
  try {
    localStorage.setItem(FAV_KEY, JSON.stringify(next));
  } catch {
    // ignore
  }
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

// Right sidebar: open pages/blocks in a side pane (shift-click a link).
export const [rightSidebar, setRightSidebar] = createSignal<
  { kind: "page" | "block"; ref: string }[]
>([]);
export function openInRightSidebar(kind: "page" | "block", ref: string) {
  const cur = rightSidebar();
  if (cur.some((i) => i.kind === kind && i.ref === ref)) return;
  setRightSidebar([{ kind, ref }, ...cur]);
}
export function closeRightSidebarItem(idx: number) {
  setRightSidebar(rightSidebar().filter((_, i) => i !== idx));
}

// Right-click block context menu.
export const [contextMenu, setContextMenu] = createSignal<{
  x: number;
  y: number;
  blockId: string;
} | null>(null);
export function openContextMenu(x: number, y: number, blockId: string) {
  setContextMenu({ x, y, blockId });
}
export function closeContextMenu() {
  setContextMenu(null);
}

export const [switcherOpen, setSwitcherOpen] = createSignal(false);
export function openSwitcher() {
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

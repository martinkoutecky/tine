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

// Opening / switching the active graph from the UI (native folder picker),
// persisting the choice so it reopens next launch.

import { backend } from "./backend";
import { setGraphMeta, setWorkflow, bumpGraphEpoch, setRightSidebar, graphMeta } from "./ui";
import { resetStore } from "./store";
import { openJournals } from "./router";
import { journalTitle } from "./journal";
import type { BlockDto } from "./types";

const GRAPH_KEY = "tine.graphPath";

export function persistedGraphPath(): string {
  try {
    return localStorage.getItem(GRAPH_KEY) ?? "";
  } catch {
    return "";
  }
}

/** Load a graph by path ("" → backend uses env/CLI). Updates meta, persists a
 *  non-empty path, and reloads the views. */
export async function loadGraphPath(path: string): Promise<void> {
  const meta = await backend().loadGraph(path);
  // New graph → drop the old graph's working set and any open sidebar items.
  resetStore();
  setRightSidebar([]);
  setGraphMeta(meta ?? null);
  setWorkflow(meta?.preferred_workflow === "todo" ? "todo" : "now");
  if (path) {
    try {
      localStorage.setItem(GRAPH_KEY, path);
    } catch {
      // ignore
    }
  }
  bumpGraphEpoch();
  void injectCustomCss();
  await ensureJournalTemplate();
  openJournals();
}

// Resolve Logseq dynamic template vars in a template block.
function applyTemplateVars(raw: string): string {
  return raw.replace(/<%\s*(today|yesterday|tomorrow|time|current time)\s*%>/gi, (_m, kw) => {
    const k = String(kw).toLowerCase();
    const d = new Date();
    if (k === "time" || k === "current time") {
      return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    }
    if (k === "yesterday") d.setDate(d.getDate() - 1);
    if (k === "tomorrow") d.setDate(d.getDate() + 1);
    return `[[${journalTitle(d)}]]`;
  });
}

// If config.edn sets :default-templates {:journals "X"}, create today's journal
// from that template when it doesn't exist yet (or is empty). No-op when unset,
// so default behaviour is unchanged.
async function ensureJournalTemplate(): Promise<void> {
  const tname = graphMeta()?.default_journal_template;
  if (!tname) return;
  const title = journalTitle(new Date());
  try {
    const existing = await backend().getPage(title, "journal");
    if (existing && existing.blocks.some((b) => b.raw.trim() !== "")) return; // already has content
    const tmpl = (await backend().listTemplates()).find((t) => t.name === tname);
    if (!tmpl) return;
    const resolve = (b: BlockDto): BlockDto => ({
      id: "",
      raw: applyTemplateVars(b.raw),
      collapsed: false,
      children: b.children.map(resolve),
    });
    await backend().savePage(
      { name: title, kind: "journal", title, pre_block: null, blocks: tmpl.blocks.map(resolve) },
      false
    );
  } catch {
    // ignore — never block graph open on template insertion
  }
}

/** Load the graph's logseq/custom.css into a <style> tag (user theming). */
async function injectCustomCss(): Promise<void> {
  let css = "";
  try {
    css = await backend().readCustomCss();
  } catch {
    css = "";
  }
  let el = document.getElementById("tine-custom-css");
  if (!el) {
    el = document.createElement("style");
    el.id = "tine-custom-css";
    document.head.appendChild(el);
  }
  el.textContent = css;
}

/** Pick a folder and open it as the graph. No-op if cancelled. */
export async function switchGraph(): Promise<void> {
  const path = await backend().pickFolder();
  if (path) await loadGraphPath(path);
}

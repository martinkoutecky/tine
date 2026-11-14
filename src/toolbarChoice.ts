import { createSignal } from "solid-js";
import { backend } from "./backend";
import { msg, render, type Msg } from "./i18n";
import { loadPreference, writePreference } from "./preferenceWrites";

/** Optional toolbar vocabulary, labels and default order have one owner.
 * Customization deliberately goes beyond Logseq (D-20261113-UI, #926). */
const labels = {
  calendar: msg("topbar-overflow-menu-calendar"),
  journals: msg("topbar-overflow-menu-journals"),
  share: msg("topbar-overflow-menu-share"),
  theme: msg("topbar-overflow-menu-toggle-theme"),
  "right-sidebar": msg("topbar-overflow-menu-toggle-right-sidebar"),
  workspaces: msg("workspace-switcher-workspaces"),
  settings: msg("settings-ui-settings"),
};
export type ToolbarActionId = keyof typeof labels;
export const toolbarActions = Object.entries(labels).map(([id, label]) => ({ id: id as ToolbarActionId, label }));
/** O(1) catalog lookup shared by menu rows and the customization list. */
export function toolbarActionLabel(id: ToolbarActionId): Msg { return labels[id]; }
export type ToolbarChoice = { bar: ToolbarActionId[]; menu: ToolbarActionId[] };
const ids = toolbarActions.map(action => action.id);
const KEY = "toolbar_actions";
const [choice, setChoice] = createSignal<ToolbarChoice | null>(null);
/** Device-local choice, or null for the shipped responsive default. O(1). */
export const toolbarChoice = choice;

/** Decode the approved two-list record. Unknown ids are ignored, bar wins
 * duplicates, and newly introduced actions join the menu. O(record entries
 * plus optional actions); malformed records throw to the preference loader. */
export function decodeToolbarChoice(raw: string): ToolbarChoice | null {
  if (!raw) return null;
  const value = JSON.parse(raw);
  if (!value || !Array.isArray(value.bar) || !Array.isArray(value.menu)) throw new Error("Invalid toolbar preference");
  const known = new Set<string>(ids);
  const seen = new Set<string>();
  const take = (values: unknown[]): ToolbarActionId[] => values.filter((id): id is ToolbarActionId => {
    if (typeof id !== "string" || !known.has(id) || seen.has(id)) return false;
    seen.add(id); return true;
  });
  const bar = take(value.bar);
  const menu = take(value.menu);
  menu.push(...ids.filter(id => !seen.has(id)));
  return { bar, menu };
}

/** Apply immediately and queue one audited device write; null deletes the
 * record. Failure rolls back and toasts through the shared writer. O(buttons). */
export function saveToolbarChoice(value: ToolbarChoice | null): void {
  writePreference(choice, setChoice, value, next => backend().setAppString(KEY, next === null ? null : JSON.stringify(next)), msg("toolbar-save-failed"));
}

/** Load once at startup or on a device-preference reload. Late reads cannot
 * overwrite a newer choice. One IPC read; O(optional actions). */
export function initToolbarChoice(): Promise<void> {
  return loadPreference(choice, setChoice, () => backend().getAppString(KEY, ""), decodeToolbarChoice, render(msg("toolbar-title")));
}

export type ToolbarPlacement = ToolbarChoice & { overflow: boolean };
/** Resolve bar/menu together from one width measurement. The null choice
 * preserves TOOLBAR-390B's default tiers; saved choices overflow a right suffix
 * using measured button costs and available space. O(optional actions). */
export function placeToolbarActions(choice: ToolbarChoice | null, width: number, options: {
  tabs: boolean; windowControls: boolean; workspace: boolean; available: ToolbarActionId[];
  budget: number; costs: Partial<Record<ToolbarActionId, number>>;
}): ToolbarPlacement {
  const available = new Set(options.available);
  if (choice) {
    let used = 0;
    let count = 0;
    for (const id of choice.bar) {
      if (!available.has(id)) { count++; continue; }
      const cost = options.costs[id] ?? 32;
      if (used + cost > options.budget) break;
      used += cost; count++;
    }
    return {
      bar: choice.bar.slice(0, count).filter(id => available.has(id)),
      menu: [...choice.bar.slice(count), ...choice.menu].filter(id => available.has(id)),
      overflow: true,
    };
  }
  const collapse = width <= 332 || ((options.tabs || options.windowControls) && width <= 460);
  const bar = ids.filter(id => available.has(id) && (
    id === "settings" || (id === "workspaces" ? options.workspace && width > 386
      : id === "right-sidebar" ? width > (options.windowControls ? 460 : 250) : !collapse)
  ));
  const visible = new Set(bar);
  return { bar, menu: ids.filter(id => available.has(id) && !visible.has(id)), overflow: collapse };
}

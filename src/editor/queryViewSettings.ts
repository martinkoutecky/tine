import { msg, render, type Msg } from "../i18n";
// Query presentation presets and edits. Each setter returns a new view.

import type { AggFn, Field, SortDir, ViewSettings } from "./queryIr";

// Sort presets — the one-click sort options in the bar

export interface SortPreset {
  field: string;
  dir: SortDir;
  label: Msg;
  hint: Msg;
}
export const SORT_PRESETS: SortPreset[] = [
  { field: "modified", dir: "desc", label: msg("settings-newest-first"), hint: msg("settings-most-recent-first-journal-pages-by") },
  { field: "modified", dir: "asc", label: msg("settings-oldest-first"), hint: msg("settings-oldest-first-journal-pages-by-their") },
  { field: "priority", dir: "asc", label: msg("settings-priority-a-c"), hint: msg("settings-highest-priority-a-first-unprioritized-last") },
  { field: "page", dir: "asc", label: msg("settings-page-a-z"), hint: msg("settings-alphabetically-by-the-page-each-result-lives") },
  { field: "deadline", dir: "asc", label: msg("settings-deadline"), hint: msg("settings-soonest-deadline-first-blocks-without-a-deadline") },
  { field: "scheduled", dir: "asc", label: msg("settings-scheduled"), hint: msg("settings-soonest-scheduled-first-blocks-without-one-last") },
];

/** Friendly text for a sort — a matching preset's label, else `field ↑/↓`. */
export function sortLabel(field: string, dir: SortDir): string {
  const preset = SORT_PRESETS.find((p) => p.field === field && p.dir === dir);
  if (preset) return render(preset.label).toLowerCase();
  return `${field} ${dir === "desc" ? "↓" : "↑"}`;
}

// View settings edits (§7.6 in its P0 form) `sort-by`, `aggregate`, `group-by` and `sample` are …

export function currentSort(view: ViewSettings): { field: Field; dir: SortDir } | null {
  const first = view.sort?.[0];
  return first ? { field: first[0], dir: first[1] } : null;
}

/** Replace the whole sort list with one trimmed field, or remove it. Secondary
 *  sort keys are lost on the next save. Pure. */
export function withSort(view: ViewSettings, sort: { field: Field; dir: SortDir } | null): ViewSettings {
  const next = { ...view };
  if (sort && sort.field.trim()) next.sort = [[sort.field.trim(), sort.dir]];
  else delete next.sort;
  return next;
}

export type AggState = { agg: AggFn; field: Field | null };

export function currentAgg(view: ViewSettings): AggState | null {
  const first = view.aggregates?.[0];
  if (!first) return null;
  const [field, agg] = first;
  return { agg, field: field === "" ? null : field };
}

/** Replace every aggregate with this one (fieldless count uses an empty field),
 *  or remove them all. Pure. */
export function withAgg(view: ViewSettings, agg: AggState | null): ViewSettings {
  const next = { ...view };
  // `["", "count"]` is the whole-result count — today's fieldless `(aggregate count)` (X3).
  if (agg) next.aggregates = [[agg.agg === "count" ? "" : (agg.field ?? ""), agg.agg]];
  else delete next.aggregates;
  return next;
}

export function currentGroup(view: ViewSettings): Field | null {
  return view.group_by ?? null;
}

export function withGroup(view: ViewSettings, field: Field | null): ViewSettings {
  const next = { ...view };
  if (field && field.trim()) next.group_by = field.trim();
  else delete next.group_by;
  return next;
}


// Query presentation presets and edits. Each setter returns a new view.

import type { AggFn, Field, SortDir, ViewSettings } from "./queryIr";

// Sort presets — the one-click sort options in the bar

export interface SortPreset {
  field: string;
  dir: SortDir;
  label: string;
  hint: string;
}
export const SORT_PRESETS: SortPreset[] = [
  { field: "modified", dir: "desc", label: "Newest first", hint: "Most recent first — journal pages by their date, others by when the file was last modified" },
  { field: "modified", dir: "asc", label: "Oldest first", hint: "Oldest first — journal pages by their date, others by file modified time" },
  { field: "priority", dir: "asc", label: "Priority A→C", hint: "Highest priority ([#A]) first; unprioritized last" },
  { field: "page", dir: "asc", label: "Page A→Z", hint: "Alphabetically by the page each result lives on" },
  { field: "deadline", dir: "asc", label: "Deadline", hint: "Soonest DEADLINE first; blocks without a deadline last" },
  { field: "scheduled", dir: "asc", label: "Scheduled", hint: "Soonest SCHEDULED first; blocks without one last" },
];

/** Friendly text for a sort — a matching preset's label, else `field ↑/↓`. */
export function sortLabel(field: string, dir: SortDir): string {
  const preset = SORT_PRESETS.find((p) => p.field === field && p.dir === dir);
  if (preset) return preset.label.toLowerCase();
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


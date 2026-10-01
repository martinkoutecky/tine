// One registry-backed vocabulary for condition and display pickers. QueryListbox
// owns keyboard/ARIA behaviour; this file only builds and draws its options.
import { For, Show, createEffect, createMemo, createSignal, type Accessor, type JSX } from "solid-js";
import type { Anchor, RegistryRow } from "../editor/queryIr";
import type { BuilderLeafKind } from "../editor/queryBuilder";
import { Listbox, type ListboxBody, type ListboxOption } from "./QueryListbox";
import { effectiveTypeOf, registryRowFor } from "../editor/queryPropertyType";
import { QUERY_SORT_BUILTINS, queryColumnName, queryFieldEncodable, querySortFieldName } from "../sheet/tablePresentation";
import { SHEET_BUILTIN_FIELDS } from "../sheet/config";

export type VocabularyChoice =
  | { kind: "builtin"; leaf: BuilderLeafKind }
  | { kind: "property"; key: string; throughPage: boolean }
  | { kind: "field"; field: string };
export interface VocabularyEntry {
  id: string;
  section: "Built-in" | "Properties" | "Page" | "Use a key by name" | "Fields";
  label: string;
  choice: VocabularyChoice;
  observed?: string;
  declared?: string;
  count?: number;
  unit: "blocks" | "pages";
  preview?: string;
}

const blockBuiltins: [BuilderLeafKind, string][] = [
  ["page", "Page / tag reference"], ["task", "Task marker"], ["priority", "Priority"],
  ["scheduled", "Scheduled"], ["deadline", "Deadline"], ["between", "Between dates"],
  ["content", "Full-text search"],
];
const pageBuiltins: [BuilderLeafKind, string][] = [
  ["journal", "On journal page"], ["onPage", "On page"],
  ["namespace", "In namespace"], ["pageTags", "Page tags"],
];
const phrase = (row: RegistryRow) => {
  const type = effectiveTypeOf(row);
  return type.cardinality === "many" ? `list of ${type.type}` : type.type;
};
const matches = (needle: string, text: string) => text.toLowerCase().includes(needle);

/** Build condition options from one registry snapshot. Counts refer to the
 * chosen owner scope; builtins have no invented counts. O(keys log keys). */
export function buildVocabulary(input: { anchor: Anchor; rows: RegistryRow[] | undefined; search?: string }): VocabularyEntry[] {
  const needle = (input.search ?? "").trim().toLowerCase();
  const entries: VocabularyEntry[] = [];
  const builtin = (items: [BuilderLeafKind, string][], section: VocabularyEntry["section"]) => {
    for (const [leaf, label] of items) if (matches(needle, label)) entries.push({
      id: `builtin:${leaf}`, section, label, choice: { kind: "builtin", leaf },
      unit: section === "Page" ? "pages" : input.anchor === "page" ? "pages" : "blocks",
    });
  };
  if (input.anchor === "block") builtin(blockBuiltins, "Built-in");
  const properties = (throughPage: boolean) => {
    const unit = input.anchor === "page" || throughPage ? "pages" : "blocks";
    const count = (row: RegistryRow) => unit === "pages" ? row.count_pages : row.count_blocks;
    const rows = [...input.rows ?? []].filter((row) => count(row) > 0 && matches(needle, row.normalized_name))
      .sort((a, b) => count(b) - count(a) || a.normalized_name.localeCompare(b.normalized_name));
    for (const row of rows) entries.push({
      id: `${throughPage ? "page" : "own"}:${row.normalized_name}`,
      section: throughPage ? "Page" : "Properties", label: row.normalized_name,
      choice: { kind: "property", key: row.normalized_name, throughPage },
      observed: row.cardinality === "many" ? `list of ${row.observed_type}` : row.observed_type,
      declared: row.declared ? phrase(row) : undefined, count: count(row), unit,
      preview: row.top_values?.slice(0, 3).map(([value]) => value).join(" · "),
    });
  };
  properties(false);
  builtin(pageBuiltins, "Page");
  if (input.anchor === "block") properties(true);
  const typed = input.search?.trim();
  if (typed) {
    const row = registryRowFor(input.rows, typed);
    for (const throughPage of input.anchor === "block" ? [false, true] : [false]) {
      const unit = input.anchor === "page" || throughPage ? "pages" : "blocks";
      const count = row ? unit === "pages" ? row.count_pages : row.count_blocks : 0;
      if (input.rows && count > 0) continue;
      entries.push({ id: `novel:${unit}:${typed}`, section: "Use a key by name",
        label: `Use "${typed}" as a ${unit === "pages" ? "page" : "block"} property`,
        choice: { kind: "property", key: typed, throughPage }, count: input.rows ? count : undefined,
        observed: row ? phrase(row) : undefined, unit });
    }
  }
  return entries;
}

export type DisplaySlot = "group" | "sort" | "column" | "aggregate";
/** Fields the parser can read in a given display slot. In particular Page is
 * a valid table column (GH #606). O(registry keys). */
export function displayFieldEntries(input: { slot: DisplaySlot; rowKind: "page" | "block"; rows: RegistryRow[] | undefined; search: string; formulas?: readonly string[] }): VocabularyEntry[] {
  const { slot, rowKind, rows } = input;
  const unit = rowKind === "page" ? "pages" : "blocks";
  const out: VocabularyEntry[] = [];
  const add = (field: string, label: string, count?: number) => {
    if (matches(input.search.trim().toLowerCase(), label)) out.push({
      id: `field:${field}`, section: "Fields", label, choice: { kind: "field", field }, count, unit,
    });
  };
  const builtins = rowKind === "page" ? ["name", "kind", "day"]
    : slot === "sort" ? QUERY_SORT_BUILTINS : SHEET_BUILTIN_FIELDS;
  if (slot !== "aggregate") for (const field of builtins) {
    if (rowKind === "page" && slot === "column" && field === "name") continue;
    add(rowKind === "page" && slot === "group" ? `prop:${field}` : field,
      field === "page" ? "Page" : field);
  }
  for (const row of rows ?? []) {
    const key = row.normalized_name;
    if (key && !queryFieldEncodable(key)) continue;
    if (rowKind === "page" && row.count_pages === 0) continue;
    if (slot === "column" && queryColumnName(`prop:${key}`) === null) continue;
    if (slot === "sort" && querySortFieldName(`prop:${key}`) === null) continue;
    add(slot === "group" ? `prop:${key}` : key, key,
      unit === "pages" ? row.count_pages : row.count_blocks + row.count_pages);
  }
  if (rowKind === "block" && slot === "group") for (const name of input.formulas ?? [])
    if (queryFieldEncodable(name)) add(`formula:${name}`, name);
  return out;
}

/** Shared filter/field picker. Its list window is bounded even on graphs with
 * thousands of keys; QueryListbox keeps the active option mounted for ARIA. */
export function QueryVocabularyPicker(props: {
  id: string; anchor: Anchor; rows: Accessor<RegistryRow[] | undefined>;
  pending?: Accessor<boolean>; failure?: Accessor<Error | null>; onRetry?: () => void;
  current?: VocabularyChoice | null; placeholder?: string; rootRef?: (element: HTMLDivElement) => void;
  entries?: (search: string) => VocabularyEntry[];
  onPick: (choice: VocabularyChoice) => void;
}): JSX.Element {
  const [search, setSearch] = createSignal("");
  const entries = createMemo(() => props.entries?.(search()) ?? buildVocabulary({ anchor: props.anchor, rows: props.rows(), search: search() }));
  const map = createMemo(() => new Map(entries().map((entry) => [entry.id, entry])));
  const options = createMemo(() => {
    const out: ListboxOption[] = [];
    let last = "";
    for (const entry of entries()) {
      if (entry.section !== last) { last = entry.section; out.push({ key: `header:${last}`, label: last, header: true }); }
      out.push({ key: entry.id, label: entry.label });
    }
    return out;
  });
  return <Listbox id={props.id} label="Condition field" class="qs-vocab" filterable
    placeholder={props.placeholder ?? "Search fields and properties"}
    query={search()} onQuery={setSearch} options={options()} rootRef={props.rootRef}
    status={<>
      <Show when={props.pending?.()}><div class="qs-registry-pending" role="status">Reading this graph's properties…</div></Show>
      <Show when={props.failure?.()}>{(failure) => <div class="qs-registry-failure" role="alert">
        This graph's properties could not be read. {failure().message}
        <button class="qs-registry-retry qs-vocab-retry" onClick={props.onRetry}>Try again</button>
      </div>}</Show>
    </>}
    body={(context) => <VocabularyBody context={context} entries={map} current={props.current} />}
    onPick={(key) => { const choice = map().get(key)?.choice; if (choice) props.onPick(choice); }} />;
}

function VocabularyBody(props: { context: ListboxBody; entries: Accessor<Map<string, VocabularyEntry>>; current?: VocabularyChoice | null }): JSX.Element {
  const [start, setStart] = createSignal(0);
  let scrollEl: HTMLDivElement | undefined;
  createEffect(() => {
    const active = props.context.shown().findIndex((option) => option.key === props.context.activeKey());
    if (active < 0 || !scrollEl) return;
    const top = active * 48;
    if (top < scrollEl.scrollTop) scrollEl.scrollTop = top;
    else if (top + 48 > scrollEl.scrollTop + scrollEl.clientHeight)
      scrollEl.scrollTop = top + 48 - scrollEl.clientHeight;
  });
  const visible = createMemo(() => {
    const all = props.context.shown();
    const active = all.findIndex((option) => option.key === props.context.activeKey());
    const begin = Math.max(0, Math.min(start(), Math.max(0, all.length - 40)));
    const indices = new Set<number>();
    for (let i = begin; i < Math.min(begin + 40, all.length); i++) indices.add(i);
    if (active >= 0) indices.add(active);
    return [...indices].sort((a, b) => a - b).map((i) => ({ option: all[i], i }));
  });
  const row = (option: ListboxOption) => <Show when={!option.header}
    fallback={<div class="qs-option-section">{option.label}</div>}>
    <div class="qs-vocab-row"><button type="button" id={props.context.optionId(option.key)} role="option" class="qs-option qs-vocab-option"
            classList={{ active: props.context.activeKey() === option.key,
              current: JSON.stringify(props.entries().get(option.key)?.choice) === JSON.stringify(props.current) }}
            aria-selected={props.context.activeKey() === option.key}
            onMouseEnter={() => props.context.setActiveKey(option.key)} onClick={() => props.context.pick(option.key)}>
            {option.label}
          </button>
          <Show when={props.entries().get(option.key)}>{(entry) => <small class="qs-vocab-detail">
            {entry().declared ? `${entry().declared} (declared)` : entry().observed ?? ""}
            {entry().count !== undefined ? ` · ${entry().count} ${entry().unit}` : ""}
            {entry().preview ? ` · ${entry().preview}` : ""}
          </small>}</Show>
    </div>
  </Show>;
  return <div ref={scrollEl} id={props.context.listId} class="qs-options qs-vocab-options" role="listbox"
    aria-label={props.context.label} onScroll={(event) => setStart(Math.floor(event.currentTarget.scrollTop / 48))}>
    <Show when={props.context.shown().length > 40} fallback={<For each={props.context.shown()}>{row}</For>}>
      <div style={{ height: `${props.context.shown().length * 48}px`, position: "relative" }}>
        <For each={visible()}>{({ option, i }) => <div style={{ position: "absolute", top: `${i * 48}px`, height: "48px", width: "100%" }}>
          {row(option)}
        </div>}</For>
      </div>
    </Show>
  </div>;
}

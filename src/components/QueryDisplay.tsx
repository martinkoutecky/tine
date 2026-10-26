import { For, Show, createEffect, createSignal, createUniqueId, onCleanup, type JSX } from "solid-js";
import { VIEW_KINDS, type AggFn, type ViewSettings } from "../editor/queryIr";
import type { RegistryAccess } from "./querySheetParts";
import { QueryVocabularyPicker, displayFieldEntries, type DisplaySlot } from "./QueryVocabularyPicker";
import { registerTransientLayer } from "../transientLayers";
import { t } from "../i18n";

/** The inline Display panel edits one ViewSettings value. The host owns the
 * guarded write, so a field choice and the header switcher share one save path.
 * Opening is reported to the host so it can request the shared registry only
 * while a picker may need it; closing requires no graph work. */
export function QueryDisplay(props: {
  view: () => ViewSettings;
  apply: (view: ViewSettings) => void | Promise<boolean>;
  registry: RegistryAccess;
  rowKind: () => "block" | "page";
  formulas?: () => readonly string[];
  parentTransientId?: string;
  onOpenChange?: (open: boolean) => void;
  inline?: boolean;
  defaultColumns?: () => readonly string[];
  availableColumns?: () => readonly string[];
}): JSX.Element {
  const [open, setOpen] = createSignal(props.inline ?? false);
  createEffect(() => props.onOpenChange?.(open()));
  const [picker, setPicker] = createSignal<DisplaySlot | null>(null);
  const [sampleText, setSampleText] = createSignal(props.view().sample === undefined ? "" : String(props.view().sample));
  const id = `query-display-${createUniqueId()}`;
  let trigger: HTMLButtonElement | undefined;
  let panel: HTMLDivElement | undefined;
  createEffect(() => {
    if (!open() || props.inline) return;
    const unregister = registerTransientLayer({ id, parentId: props.parentTransientId,
      root: () => panel ?? null, trigger: () => trigger ?? null,
      dismiss: () => { setOpen(false); return true; } });
    onCleanup(unregister);
  });
  const view = props.view;
  const columns = () => view().columns ?? [...props.defaultColumns?.() ?? []];
  // Availability follows the answer; saved choices survive even while absent.
  const availableColumns = () => [...new Set(props.availableColumns?.() ?? props.defaultColumns?.() ?? [])];
  const apply = (patch: Partial<ViewSettings>) => void props.apply({ ...view(), ...patch });
  const openPanel = () => {
    if (!open()) setSampleText(view().sample === undefined ? "" : String(view().sample));
    setOpen(!open());
    setPicker(null);
  };
  const pick = (slot: DisplaySlot, field: string) => {
    if (slot === "sort") apply({ sort: [...view().sort ?? [], [field, "asc"]] });
    if (slot === "column") apply({ columns: [...new Set([...columns(), field])] });
    if (slot === "aggregate") apply({ aggregates: [...view().aggregates ?? [], [field, "sum"]] });
    if (slot === "group") apply({ group_by: field });
    setPicker(null);
  };
  const fieldPicker = (slot: DisplaySlot, label: string) => <>
    <button type="button" class="qd-add" onClick={() => setPicker(picker() === slot ? null : slot)}>{label}</button>
    <Show when={picker() === slot}>
      <QueryVocabularyPicker id={`${id}-${slot}`} anchor={props.rowKind()}
        rows={slot === "column" ? () => [] : props.registry.rows}
        pending={slot === "column" ? undefined : props.registry.pending}
        failure={slot === "column" ? undefined : props.registry.failure} onRetry={props.registry.retry}
        entries={(search) => slot === "column"
          ? availableColumns().filter(field => field.toLowerCase().includes(search.trim().toLowerCase())).map(field => ({
            id: `field:${field}`, section: "Fields" as const, label: field === "page" ? t("sheet-prose-page") : field,
            choice: { kind: "field" as const, field }, unit: props.rowKind() === "page" ? "pages" as const : "blocks" as const,
          }))
          : displayFieldEntries({ slot, rowKind: props.rowKind(), rows: props.registry.rows(),
            formulas: props.formulas?.(), search })}
        onPick={(choice) => { if (choice.kind === "field") pick(slot, choice.field); }} />
    </Show>
  </>;
  const move = <T,>(items: T[], from: number, to: number): T[] => {
    const copy = [...items];
    if (to < 0 || to >= copy.length) return copy;
    copy.splice(to, 0, copy.splice(from, 1)[0]);
    return copy;
  };
  const listRow = (label: string, index: number, length: number, moveRow: (to: number) => void, remove: () => void, extra?: JSX.Element) =>
    <div class="qd-row"><span>{label}</span>{extra}
      <button class="settings-btn" type="button" aria-label={t("query-query-display-move-up", { value1: label })} disabled={index === 0} onClick={() => moveRow(index - 1)}>↑</button>
      <button class="settings-btn" type="button" aria-label={t("query-query-display-move-down", { value1: label })} disabled={index === length - 1} onClick={() => moveRow(index + 1)}>↓</button>
      <button class="settings-btn" type="button" aria-label={t("query-query-display-remove", { value1: label })} onClick={remove}>×</button>
    </div>;
  const sample = () => {
    const text = sampleText().trim();
    if (text === "") return { valid: true, value: undefined };
    if (!/^\d+$/.test(text) || Number(text) > 4294967295) return { valid: false, value: undefined };
    return { valid: true, value: Number(text) };
  };
  const commitSample = () => {
    const parsed = sample();
    if (parsed.valid && parsed.value !== view().sample) apply({ sample: parsed.value });
  };
  return <div class="qd-root">
    <Show when={!props.inline}><button ref={trigger} type="button" class="qd-trigger" aria-expanded={open()} aria-controls={id}
      onClick={openPanel}>{props.rowKind() === "page" ? t("query-query-display-display-pages") : t("query-query-display-display")}</button></Show>
    <Show when={open()}><div ref={panel} id={id} class="qd-panel" role="dialog"
      aria-label={props.rowKind() === "page" ? t("query-query-display-page-display") : t("query-query-display-block-display")}
      onClick={(event) => event.stopPropagation()}>
      <section><h4>{t("query-query-display-view")}</h4><div role="group" aria-label={t("query-query-display-query-view")}>
        <For each={VIEW_KINDS}>{(kind) =>
          <button class="settings-btn" type="button" classList={{ active: (view().view ?? "list") === kind }}
            onClick={() => apply({ view: kind })}>{viewLabel(kind)}</button>}</For>
      </div></section>
      <section><h4>{t("query-query-display-group-by")}</h4><span>{view().group_by || t("query-query-display-no-grouping")}</span>
        <button class="settings-btn" type="button" onClick={() => apply({ group_by: "" })}>{t("query-query-display-none")}</button>
        {fieldPicker("group", t("query-display-change"))}
      </section>
      <section><h4>{t("query-query-display-sort")}</h4>
        <For each={view().sort ?? []}>{([field, dir], index) => listRow(field, index(), view().sort?.length ?? 0,
          (to) => apply({ sort: move(view().sort ?? [], index(), to) }),
          () => apply({ sort: (view().sort ?? []).filter((_, i) => i !== index()) }),
          <button class="settings-btn" type="button" aria-label={t("query-sort-direction", { field, direction: dir })}
            onClick={() => apply({ sort: (view().sort ?? []).map(([f, d], i) => i === index() ? [f, d === "asc" ? "desc" : "asc"] : [f, d]) })}>
            {dir === "asc" ? t("query-query-display-asc") : t("query-query-display-desc")}</button>)}
        </For>{fieldPicker("sort", t("query-display-add-sort"))}
      </section>
      <section aria-label={t("query-calm-columns")}><h4>{t("query-query-display-columns")}</h4>
        <Show when={view().view === "table"}><label class="qd-row">
          <input type="checkbox" checked disabled />{t("query-calm-title-column")}
        </label></Show>
        <button class="settings-btn" type="button" onClick={() => apply({ columns: availableColumns() })}>{t("query-calm-all")}</button>
        <button class="settings-btn" type="button" onClick={() => apply({ columns: [] })}>{t("query-calm-none")}</button>
        <For each={availableColumns()}>{(field) => <label class="qd-row">
          <input type="checkbox" aria-label={field} checked={columns().includes(field)} onChange={(event) =>
            apply({ columns: event.currentTarget.checked ? [...columns(), field] : columns().filter((column) => column !== field) })} />
          {field}
        </label>}</For>
        {fieldPicker("column", t("query-display-add-column"))}
      </section>
      <section><h4>{t("query-query-display-summarize")}</h4>
        <For each={view().aggregates ?? []}>{([field, fn], index) => listRow(field || t("query-display-whole-result"), index(), view().aggregates?.length ?? 0,
          (to) => apply({ aggregates: move(view().aggregates ?? [], index(), to) }),
          () => apply({ aggregates: (view().aggregates ?? []).filter((_, i) => i !== index()) }),
          <button class="settings-btn" type="button" onClick={() => {
            const cycle: AggFn[] = ["count", "sum", "avg"];
            apply({ aggregates: (view().aggregates ?? []).map(([f, a], i) => i === index()
              ? [f, cycle[(cycle.indexOf(a) + 1) % cycle.length]] : [f, a]) });
          }}>{aggregateLabel(fn)}</button>)}
        </For>
        <button class="settings-btn" type="button" onClick={() => apply({ aggregates: [...view().aggregates ?? [], ["", "count"]] })}>{t("query-query-display-count")}</button>
        {fieldPicker("aggregate", t("query-display-add-property"))}
      </section>
      <section><h4>{t("query-query-display-sample")}</h4><input aria-label={t("query-query-display-sample-size")} inputmode="numeric" placeholder={t("query-query-display-no-limit")}
        value={sampleText()} onInput={(event) => setSampleText(event.currentTarget.value)}
        onBlur={commitSample} onKeyDown={(event) => { if (event.key === "Enter") commitSample(); }} />
        <Show when={!sample().valid}><div role="alert">{t("query-query-display-a-sample-is-a-whole-number-up-to-4294967295")}</div></Show>
        <Show when={sample().valid && sample().value === 0}><div>{t("query-query-display-no-results-sample-0")}</div></Show>
      </section>
      <Show when={!props.inline}><button class="settings-btn" type="button" onClick={() => setOpen(false)}>{t("query-query-display-done")}</button></Show>
    </div></Show>
  </div>;
}

function viewLabel(kind: typeof VIEW_KINDS[number]): string {
  switch (kind) {
    case "search": return t("query-display-view-search");
    case "list": return t("query-display-view-list");
    case "table": return t("query-display-view-table");
    case "board": return t("query-display-view-board");
  }
}
function aggregateLabel(fn: AggFn): string {
  switch (fn) {
    case "count": return t("query-display-aggregate-count");
    case "sum": return t("query-display-aggregate-sum");
    case "avg": return t("query-display-aggregate-avg");
  }
}

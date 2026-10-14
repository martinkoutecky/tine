import { For, Show, createSignal, type JSX } from "solid-js";
import type { CmpOp, Filter } from "../editor/queryIr";
import { t } from "../i18n";
import { previewDate } from "../editor/dateExpr";

/** Emits existing date leaves. SQL resolves and compares the literal. */
export function PlanningDatePick(props: { field: "scheduled" | "deadline"; onCommit: (filter: Filter) => void }): JSX.Element {
  const [op, setOp] = createSignal<CmpOp>("is_set");
  const [date, setDate] = createSignal("today");
  const presence = () => op() === "is_set" || op() === "is_not_set";
  const commit = () => props.onCommit({ kind: "leaf", leaf: { kind: "attr", attr: props.field, op: op(),
    value: presence() ? { kind: "none" } : { kind: "date", literal: date().trim() } } });
  return <div class="qs-value-editor">
    <select aria-label={t("query-calm-date-comparison")} value={op()} onChange={(event) => setOp(event.currentTarget.value as CmpOp)}>
      <option value="is_set">{t("query-calm-is-set")}</option>
      <option value="is_not_set">{t("query-calm-is-not-set")}</option>
      <For each={[["gt", ">"], ["ge", "≥"], ["eq", "="], ["lt", "<"], ["le", "≤"]] as const}>
        {([value, label]) => <option value={value}>{label}</option>}
      </For>
    </select>
    <Show when={!presence()}>
      <div class="qs-between-presets"><For each={["today", "yesterday", "tomorrow"] as const}>
        {(value) => <button type="button" onClick={() => setDate(value)}>{t(`query-calm-${value}`)}</button>}
      </For></div>
      <input class="qs-input" aria-label={t("query-calm-date")} value={date()}
        placeholder={t("query-calm-relative-date")} onInput={(event) => setDate(event.currentTarget.value)}
        onKeyDown={(event) => { if (event.key === "Enter" && date().trim()) commit(); }} />
      <span class="qs-bound-preview">{previewDate(date())}</span>
    </Show>
    <button type="button" class="qs-commit" disabled={!presence() && !date().trim()} onClick={commit}>{t("query-calm-apply")}</button>
  </div>;
}

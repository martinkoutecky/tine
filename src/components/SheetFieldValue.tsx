import { For, Match, Show, Switch, type JSX } from "solid-js";
import { formatForPage } from "../document";
import { InlineText } from "../render/inline";
import { isFormulaField, type FieldId, type FieldValue } from "../sheet/fields";
import { type FieldType } from "../sheet/config";
import { formulaValueText } from "../sheet/formulaEval";
import type { FormulaValue } from "../sheet/formula";
import { parseIsoDateLike } from "../sheet/typed";
import { markerLabelClickable } from "../editor/repeat";

/** Render one field value in its declared presentation; read-only and O(value length). */
export function FieldValueView(props: {
  field: FieldId;
  fieldType?: FieldType;
  value: FieldValue | null;
  formulaValue?: FormulaValue | null;
  page: string;
  onControlClick?: (e: MouseEvent) => void;
}): JSX.Element {
  if (isFormulaField(props.field)) return <FormulaValueView value={props.formulaValue ?? null} />;
  const text = () => props.value?.text ?? "";
  const stopControlDoubleClick = (e: MouseEvent) => {
    if (!props.onControlClick) return;
    e.preventDefault();
    e.stopPropagation();
  };
  return (
    <Show when={props.value}>
      <Show when={props.field === "state"}>
        <span
          class={`block-marker marker-${(props.value?.raw ?? "").toLowerCase()}`}
          classList={{ "marker-clickable": markerLabelClickable(props.value?.raw) }}
          onClick={props.onControlClick}
          onDblClick={stopControlDoubleClick}
        >
          {props.value?.text}
        </span>
      </Show>
      <Show when={props.field === "priority"}>
        <span
          class={`block-priority priority-${props.value?.raw}`}
          onClick={props.onControlClick}
          onDblClick={stopControlDoubleClick}
        >
          {props.value?.text}
        </span>
      </Show>
      <Show when={props.field === "scheduled"}>
        <span class="date-chip scheduled" onClick={props.onControlClick} onDblClick={stopControlDoubleClick}>{text()}</span>
      </Show>
      <Show when={props.field === "deadline"}>
        <span class="date-chip deadline" onClick={props.onControlClick} onDblClick={stopControlDoubleClick}>{text()}</span>
      </Show>
      <Show when={props.field === "tags"}>
        <For each={(props.value?.raw ?? "").split(/\s+/).filter(Boolean)}>
          {(tag) => <span class="sheet-tag-chip">#{tag}</span>}
        </For>
      </Show>
      <Show when={props.field.startsWith("prop:")}>
        <PropValueView type={props.fieldType} value={props.value!} page={props.page} onControlClick={props.onControlClick} />
      </Show>
      <Show when={props.field === "page"}>
        <InlineText text={text()} format={formatForPage(props.page)} />
      </Show>
    </Show>
  );
}

function FormulaValueView(props: { value: FormulaValue | null }): JSX.Element {
  return (
    <Switch>
      <Match when={props.value?.kind === "error"}>
        <span class="sheet-formula-error" title={props.value?.kind === "error" ? props.value.message : ""}>
          ⚠
        </span>
      </Match>
      <Match when={props.value?.kind === "number"}>
        {formulaValueText(props.value)}
      </Match>
      <Match when={props.value?.kind === "date"}>
        <span class="date-chip scheduled">{formulaValueText(props.value)}</span>
      </Match>
      <Match when={props.value?.kind === "boolean"}>
        <input
          class="sheet-checkbox"
          type="checkbox"
          checked={props.value?.kind === "boolean" ? props.value.value : false}
          disabled
        />
      </Match>
      <Match when={props.value?.kind === "list"}>
        <For each={props.value?.kind === "list" ? props.value.values : []}>
          {(value) => <span class="sheet-tag-chip">{formulaValueText(value)}</span>}
        </For>
      </Match>
      <Match when={props.value?.kind === "text" || props.value?.kind === "duration"}>
        {formulaValueText(props.value)}
      </Match>
    </Switch>
  );
}

function PropValueView(props: { type?: FieldType; value: FieldValue; page: string; onControlClick?: (e: MouseEvent) => void }): JSX.Element {
  const text = () => props.value.text;
  const raw = () => props.value.raw ?? props.value.text;
  const stopControlDoubleClick = (e: MouseEvent) => {
    if (!props.onControlClick) return;
    e.preventDefault();
    e.stopPropagation();
  };
  const checkbox = () => {
    if (props.type !== "checkbox") return null;
    const lower = raw().trim().toLowerCase();
    if (lower === "true") return true;
    if (lower === "false") return false;
    return null;
  };
  const dateValue = () => {
    if (props.type !== "date" && props.type !== "datetime") return null;
    const value = raw().trim();
    return validDateLike(value) ? value : null;
  };
  const enumValue = () => {
    if (!isEnumFieldType(props.type)) return null;
    const value = raw().trim();
    return props.type.enum.includes(value) ? value : null;
  };
  const listValues = () =>
    props.type === "list"
      ? raw()
          .split(",")
          .map((v) => v.trim())
          .filter(Boolean)
      : [];
  const refValue = () => {
    if (props.type !== "ref") return null;
    const value = raw().trim();
    return /^\[\[[^\]\n\r]+\]\]$/.test(value) ? value : null;
  };
  return (
    <Switch fallback={<InlineText text={text()} format={formatForPage(props.page)} />}>
      <Match when={checkbox() !== null}>
        <input
          class="sheet-checkbox"
          type="checkbox"
          checked={checkbox() === true}
          readOnly
          onClick={props.onControlClick}
          onDblClick={stopControlDoubleClick}
        />
      </Match>
      <Match when={dateValue()}>
        {(value) => <span class="date-chip scheduled" onClick={props.onControlClick} onDblClick={stopControlDoubleClick}>{value()}</span>}
      </Match>
      <Match when={enumValue()}>
        {(value) => <span class="sheet-tag-chip" onClick={props.onControlClick} onDblClick={stopControlDoubleClick}>{value()}</span>}
      </Match>
      <Match when={props.type === "list" && listValues().length > 0}>
        <For each={listValues()}>{(value) => <span class="sheet-tag-chip">{value}</span>}</For>
      </Match>
      <Match when={refValue()}>
        {(value) => <InlineText text={value()} format={formatForPage(props.page)} />}
      </Match>
    </Switch>
  );
}

/** True for a declared enumerated field type. */
export function isEnumFieldType(type: FieldType | undefined): type is { enum: readonly string[] } {
  return typeof type === "object" && type !== null && "enum" in type;
}

function validDateLike(value: string): boolean {
  return parseIsoDateLike(value) !== null;
}

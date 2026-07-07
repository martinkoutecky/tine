import { For, Show, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import {
  closeFormulaEditor,
  formulaEditor,
  type FormulaEditorHome,
  type FormulaEditorTarget,
} from "../ui";
import { blockPageReadOnly, doc, pageByName, setBlockProperty, setPageProperty } from "../store";
import { encodeFormulaExpr, formulaNameValid, parseFormula } from "../sheet/formula";

const STDLIB_CHIPS = [
  "if()",
  "isEmpty()",
  "now()",
  "today()",
  ".contains()",
  ".lower()",
  ".trim()",
  ".replace()",
  ".length",
  ".round()",
  ".floor()",
  ".ceil()",
  ".abs()",
  ".toFixed()",
  ".format()",
  ".year",
  ".month",
  ".day",
  ".relative()",
  ".join()",
];

export function FormulaEditor(): JSX.Element {
  return (
    <Show when={formulaEditor()} keyed>
      {(target) => <FormulaEditorPopup target={target} />}
    </Show>
  );
}

function FormulaEditorPopup(props: { target: FormulaEditorTarget }): JSX.Element {
  const [name, setName] = createSignal(props.target.mode === "add" ? "" : props.target.name ?? "");
  const [expr, setExpr] = createSignal(props.target.expr);
  const [winW, setWinW] = createSignal(typeof window !== "undefined" ? window.innerWidth : 1280);
  const [winH, setWinH] = createSignal(typeof window !== "undefined" ? window.innerHeight : 800);
  let firstInput: HTMLInputElement | HTMLTextAreaElement | undefined;
  let textarea: HTMLTextAreaElement | undefined;

  const existingNames = createMemo(() => new Set(props.target.formulas.map(([n]) => n)));
  const formulaChips = createMemo(() => props.target.formulas.map(([n]) => `formula.${n}`));
  const modeLabel = () =>
    props.target.mode === "filter" ? "Edit filter" : props.target.mode === "edit" ? "Edit formula" : "Add formula";

  const parseResult = createMemo(() => {
    const value = expr().trim();
    if (props.target.mode === "filter" && value === "") return { ok: true as const };
    return parseFormula(value);
  });
  const parseError = () => {
    const parsed = parseResult();
    return parsed.ok ? null : parsed.error;
  };
  const nameError = () => {
    if (props.target.mode !== "add") return null;
    const value = name().trim();
    if (!formulaNameValid(value)) return "Use lowercase letters, digits, and hyphens.";
    if (existingNames().has(value)) return "A formula with this name already exists.";
    return null;
  };
  const home = (): FormulaEditorHome | null => {
    if (props.target.mode === "edit") return props.target.home ?? null;
    if (props.target.mode === "filter") return doc.byId[props.target.ownerId] ? { kind: "block", id: props.target.ownerId } : null;
    if (props.target.home) return props.target.home;
    if (doc.byId[props.target.ownerId]) return { kind: "block", id: props.target.ownerId };
    return props.target.schemaPage ? { kind: "page", name: props.target.schemaPage } : null;
  };
  const writeAllowed = () => {
    const h = home();
    if (!h) return false;
    if (h.kind === "block") return !blockPageReadOnly(h.id);
    return !(pageByName(h.name)?.readOnly ?? false);
  };
  const canSave = () => writeAllowed() && !nameError() && !parseError();

  const markerLine = () => {
    const err = parseError();
    if (!err) return "";
    const before = expr().slice(0, err.offset);
    const col = before.split("\n").at(-1)?.length ?? 0;
    return `${" ".repeat(col)}^`;
  };
  const left = () => Math.max(4, Math.min(props.target.x, winW() - 460));
  const top = () => Math.max(4, Math.min(props.target.y, winH() - 380));

  const insertAtCaret = (text: string) => {
    const start = textarea?.selectionStart ?? expr().length;
    const end = textarea?.selectionEnd ?? start;
    const next = `${expr().slice(0, start)}${text}${expr().slice(end)}`;
    setExpr(next);
    queueMicrotask(() => {
      textarea?.focus();
      textarea?.setSelectionRange(start + text.length, start + text.length);
    });
  };
  const save = () => {
    if (!canSave()) return;
    const h = home();
    if (!h) return;
    const value = expr().trim();
    if (props.target.mode === "filter") {
      if (h.kind === "block") setBlockProperty(h.id, "tine.filter", value ? encodeFormulaExpr(value) : null);
      closeFormulaEditor();
      return;
    }

    const formulaName = props.target.mode === "add" ? name().trim() : props.target.name ?? "";
    if (!formulaName) return;
    const key = `tine.formula.${formulaName}`;
    if (h.kind === "block") setBlockProperty(h.id, key, encodeFormulaExpr(value));
    else setPageProperty(h.name, key, encodeFormulaExpr(value));
    closeFormulaEditor();
  };

  onMount(() => {
    queueMicrotask(() => firstInput?.focus());
    const onResize = () => {
      setWinW(window.innerWidth);
      setWinH(window.innerHeight);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeFormulaEditor();
      }
    };
    window.addEventListener("resize", onResize);
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => {
      window.removeEventListener("resize", onResize);
      window.removeEventListener("keydown", onKey, true);
    });
  });

  return (
    <div
      class="formula-editor-overlay"
      onClick={closeFormulaEditor}
      onContextMenu={(e) => {
        e.preventDefault();
        closeFormulaEditor();
      }}
    >
      <form
        class="formula-editor"
        style={{ left: `${left()}px`, top: `${top()}px` }}
        onClick={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <div class="formula-editor-head">{modeLabel()}</div>
        <Show when={props.target.mode === "add"}>
          <label class="formula-editor-label">
            Name
            <input
              class="formula-editor-input"
              classList={{ invalid: !!nameError() }}
              ref={(el) => {
                firstInput = el;
              }}
              value={name()}
              onInput={(e) => setName(e.currentTarget.value)}
            />
          </label>
          <Show when={nameError()}>
            {(err) => <div class="formula-editor-error">{err()}</div>}
          </Show>
        </Show>
        <label class="formula-editor-label">
          Expression
          <textarea
            class="formula-editor-textarea"
            classList={{ invalid: !!parseError() }}
            ref={(el) => {
              textarea = el;
              if (props.target.mode !== "add") firstInput = el;
            }}
            value={expr()}
            onInput={(e) => setExpr(e.currentTarget.value)}
          />
        </label>
        <Show when={parseError()}>
          {(err) => (
            <div class="formula-editor-error">
              <span>{err().message}</span>
              <pre>{markerLine()}</pre>
            </div>
          )}
        </Show>
        <div class="formula-editor-hints">
          <For each={props.target.fields}>
            {(field) => (
              <button type="button" class="formula-editor-chip" onClick={() => insertAtCaret(field)}>
                {field}
              </button>
            )}
          </For>
          <For each={formulaChips()}>
            {(formula) => (
              <button type="button" class="formula-editor-chip" onClick={() => insertAtCaret(formula)}>
                {formula}
              </button>
            )}
          </For>
          <For each={STDLIB_CHIPS}>
            {(fn) => (
              <button type="button" class="formula-editor-chip" onClick={() => insertAtCaret(fn)}>
                {fn}
              </button>
            )}
          </For>
        </div>
        <div class="formula-editor-actions">
          <button type="button" class="formula-editor-btn" onClick={closeFormulaEditor}>
            Cancel
          </button>
          <button type="submit" class="formula-editor-btn primary" disabled={!canSave()}>
            Save
          </button>
        </div>
      </form>
    </div>
  );
}

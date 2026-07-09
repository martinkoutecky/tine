import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import {
  closeFormulaEditor,
  formulaEditor,
  type FormulaEditorHome,
  type FormulaEditorTarget,
} from "../ui";
import { blockPageReadOnly, doc, pageByName, setBlockProperty, setPageProperty } from "../store";
import { astToExpr, encodeFormulaExpr, formulaNameValid, parseFormula, type Ast, type BinaryOp } from "../sheet/formula";

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

type EditorParseResult = { ok: true; ast: Ast | null } | { ok: false; error: { offset: number; message: string } };

const COMPARISON_OPS: BinaryOp[] = ["==", "!=", "<", "<=", ">", ">="];
const BOOLEAN_OPS: BinaryOp[] = ["&&", "||"];
const ARITHMETIC_OPS: BinaryOp[] = ["+", "-", "*", "/", "%"];
const CONDITION_OPS = new Set<BinaryOp>([...COMPARISON_OPS, ...BOOLEAN_OPS]);
const VALUE_OPS = new Set<BinaryOp>(ARITHMETIC_OPS);
const OPERATOR_OPTIONS: { op: BinaryOp; label: string }[] = [
  { op: "==", label: "=" },
  { op: "!=", label: "≠" },
  { op: "<", label: "<" },
  { op: "<=", label: "≤" },
  { op: ">", label: ">" },
  { op: ">=", label: "≥" },
  { op: "&&", label: "and" },
  { op: "||", label: "or" },
  { op: "+", label: "+" },
  { op: "-", label: "−" },
  { op: "*", label: "×" },
  { op: "/", label: "÷" },
  { op: "%", label: "mod" },
];

type TransformArg = { kind: "text" | "number"; placeholder: string; value: string };
type TransformSpec = {
  name: string;
  label: string;
  group: "text" | "number" | "date" | "any";
  property?: boolean;
  args: TransformArg[];
};

const TRANSFORMS: TransformSpec[] = [
  { name: "contains", label: "contains text", group: "text", args: [{ kind: "text", placeholder: "needle", value: "" }] },
  { name: "lower", label: "lowercase", group: "text", args: [] },
  { name: "trim", label: "trim spaces", group: "text", args: [] },
  {
    name: "replace",
    label: "replace text",
    group: "text",
    args: [
      { kind: "text", placeholder: "search", value: "" },
      { kind: "text", placeholder: "replacement", value: "" },
    ],
  },
  { name: "join", label: "join list", group: "text", args: [{ kind: "text", placeholder: "separator", value: "," }] },
  { name: "round", label: "round", group: "number", args: [] },
  { name: "floor", label: "floor", group: "number", args: [] },
  { name: "ceil", label: "ceil", group: "number", args: [] },
  { name: "abs", label: "absolute", group: "number", args: [] },
  { name: "toFixed", label: "fixed decimals", group: "number", args: [{ kind: "number", placeholder: "places", value: "0" }] },
  { name: "year", label: "year", group: "date", property: true, args: [] },
  { name: "month", label: "month", group: "date", property: true, args: [] },
  { name: "day", label: "day", group: "date", property: true, args: [] },
  { name: "relative", label: "relative date", group: "date", args: [] },
  { name: "format", label: "format date", group: "date", args: [{ kind: "text", placeholder: "format", value: "YYYY-MM-DD" }] },
  { name: "length", label: "length", group: "any", property: true, args: [] },
];

const TRANSFORM_BY_NAME = new Map(TRANSFORMS.map((spec) => [spec.name, spec]));

function isConditional(ast: Ast): ast is Ast & { kind: "call"; args: [Ast, Ast, Ast] } {
  return ast.kind === "call" && ast.name === "if" && ast.args.length === 3;
}

function isConditionAst(ast: Ast): boolean {
  return ast.kind === "binary" && CONDITION_OPS.has(ast.op);
}

function isKnownMember(ast: Ast): boolean {
  if (ast.kind !== "member") return false;
  const spec = TRANSFORM_BY_NAME.get(ast.name);
  if (!spec) return false;
  if (spec.property) return ast.args == null;
  return ast.args != null && ast.args.length === spec.args.length;
}

function isSimpleValueAst(ast: Ast): boolean {
  if (ast.kind === "literal" || ast.kind === "field" || ast.kind === "formulaRef") return true;
  if (ast.kind === "call") return (ast.name === "now" || ast.name === "today") && ast.args.length === 0;
  if (ast.kind === "member") return isKnownMember(ast) && isSimpleValueAst(ast.object) && ast.args?.every(isSimpleValueAst) !== false;
  return false;
}

function isValueAst(ast: Ast): boolean {
  if (isSimpleValueAst(ast)) return true;
  return ast.kind === "binary" && VALUE_OPS.has(ast.op) && isSimpleValueAst(ast.left) && isSimpleValueAst(ast.right);
}

function formulaBuilderCanRenderRoot(ast: Ast): boolean {
  return isConditional(ast) || isConditionAst(ast) || isValueAst(ast);
}

function literalLabel(value: string | number | boolean | null): string {
  if (typeof value === "string") return `"${value}"`;
  return value === null ? "empty" : String(value);
}

function opLabel(op: BinaryOp): string {
  return OPERATOR_OPTIONS.find((option) => option.op === op)?.label ?? op;
}

function valueLabel(ast: Ast): string {
  switch (ast.kind) {
    case "literal":
      return literalLabel(ast.value);
    case "field":
      return ast.name;
    case "formulaRef":
      return `formula.${ast.name}`;
    case "call":
      return `${ast.name}()`;
    case "member":
      return astToExpr(ast);
    case "binary":
      return `${valueLabel(ast.left)} ${opLabel(ast.op)} ${valueLabel(ast.right)}`;
    case "unary":
      return astToExpr(ast);
  }
}

function parseAstText(text: string): Ast | null {
  const parsed = parseFormula(text.trim());
  return parsed.ok ? parsed.ast : null;
}

function transformAst(object: Ast, spec: TransformSpec, values: readonly string[]): Ast {
  return {
    kind: "member",
    object,
    name: spec.name,
    args: spec.property
      ? null
      : spec.args.map((arg, i): Ast => ({
          kind: "literal",
          value: arg.kind === "number" ? Number(values[i] || 0) : values[i] ?? "",
        })),
  };
}

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
  const initialParsed = (() => {
    const value = props.target.expr.trim();
    if (props.target.mode === "filter" && value === "") return null;
    const parsed = parseFormula(value);
    return parsed.ok ? parsed.ast : null;
  })();
  const [rawMode, setRawMode] = createSignal(initialParsed == null);
  const [winW, setWinW] = createSignal(typeof window !== "undefined" ? window.innerWidth : 1280);
  const [winH, setWinH] = createSignal(typeof window !== "undefined" ? window.innerHeight : 800);
  let firstInput: HTMLInputElement | HTMLTextAreaElement | undefined;
  let textarea: HTMLTextAreaElement | undefined;

  const existingNames = createMemo(() => new Set(props.target.formulas.map(([n]) => n)));
  const formulaChips = createMemo(() => props.target.formulas.map(([n]) => `formula.${n}`));
  const modeLabel = () =>
    props.target.mode === "filter" ? "Edit filter" : props.target.mode === "edit" ? "Edit formula" : "Add formula";

  const parseResult = createMemo((): EditorParseResult => {
    const value = expr().trim();
    if (props.target.mode === "filter" && value === "") return { ok: true, ast: null };
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
  const ast = () => {
    const parsed = parseResult();
    return parsed.ok ? parsed.ast : null;
  };
  const showBuilder = () => !rawMode() && ast() != null;
  const toggleRawMode = () => {
    if (!rawMode()) {
      setRawMode(true);
      queueMicrotask(() => textarea?.focus());
      return;
    }
    const parsed = parseResult();
    if (parsed.ok && parsed.ast) setRawMode(false);
  };

  const insertAtCaret = (text: string) => {
    if (!rawMode()) setRawMode(true);
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
        <div class="formula-editor-label">
          <div class="formula-editor-label-row">
            <span>Expression</span>
            <button
              type="button"
              class="qb-sort formula-editor-raw-toggle"
              classList={{ active: rawMode() }}
              title={rawMode() ? "Switch to visual formula builder" : "Edit raw formula text"}
              onClick={toggleRawMode}
            >
              {rawMode() ? "Builder" : "</> raw"}
            </button>
          </div>
          <Show
            keyed
            when={showBuilder() && ast()}
            fallback={
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
            }
          >
            {(node) => (
              <FormulaBuilder
                ast={node}
                source={expr()}
                fields={props.target.fields}
                formulas={props.target.formulas}
                onExpr={setExpr}
                onAst={(next) => setExpr(astToExpr(next))}
              />
            )}
          </Show>
        </div>
        <Show when={parseError()}>
          {(err) => (
            <div class="formula-editor-error">
              <span>{err().message}</span>
              <pre>{markerLine()}</pre>
            </div>
          )}
        </Show>
        <Show when={rawMode()}>
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
        </Show>
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

function FormulaBuilder(props: {
  ast: Ast;
  source: string;
  fields: readonly string[];
  formulas: readonly [string, string][];
  onExpr: (expr: string) => void;
  onAst: (ast: Ast) => void;
}): JSX.Element {
  return (
    <div class="formula-builder qb-bar">
      <Show
        when={formulaBuilderCanRenderRoot(props.ast)}
        fallback={<RootRawExpressionFace source={props.source} onExpr={props.onExpr} />}
      >
        <RootFormulaFace
          ast={props.ast}
          source={props.source}
          fields={props.fields}
          formulas={props.formulas}
          onAst={props.onAst}
          onExpr={props.onExpr}
        />
      </Show>
    </div>
  );
}

function RootFormulaFace(props: {
  ast: Ast;
  source: string;
  fields: readonly string[];
  formulas: readonly [string, string][];
  onAst: (ast: Ast) => void;
  onExpr: (expr: string) => void;
}): JSX.Element {
  if (isConditional(props.ast)) {
    return (
      <ConditionalFace
        ast={props.ast}
        fields={props.fields}
        formulas={props.formulas}
        onChange={props.onAst}
      />
    );
  }
  if (isConditionAst(props.ast)) {
    return (
      <ConditionFace
        ast={props.ast}
        fields={props.fields}
        formulas={props.formulas}
        onChange={props.onAst}
      />
    );
  }
  if (isValueAst(props.ast)) {
    return (
      <ValueFace
        ast={props.ast}
        fields={props.fields}
        formulas={props.formulas}
        onChange={props.onAst}
      />
    );
  }
  return <RootRawExpressionFace source={props.source} onExpr={props.onExpr} />;
}

function ConditionalFace(props: {
  ast: Ast & { kind: "call"; args: [Ast, Ast, Ast] };
  fields: readonly string[];
  formulas: readonly [string, string][];
  onChange: (ast: Ast) => void;
}): JSX.Element {
  const setArg = (index: 0 | 1 | 2, value: Ast) => {
    const args = [...props.ast.args];
    args[index] = value;
    props.onChange({ ...props.ast, args });
  };
  return (
    <div class="formula-builder-if">
      <span class="formula-builder-keyword">IF</span>
      <ConditionFace
        ast={props.ast.args[0]}
        fields={props.fields}
        formulas={props.formulas}
        onChange={(next) => setArg(0, next)}
      />
      <span class="formula-builder-keyword">THEN</span>
      <ValueFace
        ast={props.ast.args[1]}
        fields={props.fields}
        formulas={props.formulas}
        onChange={(next) => setArg(1, next)}
      />
      <span class="formula-builder-keyword">ELSE</span>
      <ValueFace
        ast={props.ast.args[2]}
        fields={props.fields}
        formulas={props.formulas}
        onChange={(next) => setArg(2, next)}
      />
    </div>
  );
}

function ConditionFace(props: {
  ast: Ast;
  fields: readonly string[];
  formulas: readonly [string, string][];
  onChange: (ast: Ast) => void;
}): JSX.Element {
  if (props.ast.kind !== "binary" || !CONDITION_OPS.has(props.ast.op)) {
    return (
      <AstRawExpressionFace
        source={astToExpr(props.ast)}
        onAst={props.onChange}
      />
    );
  }
  // Narrow to the binary node once: TS drops the guard's narrowing inside the
  // closures below (props.ast is a mutable union member), so capture it in a const.
  const ast = props.ast;
  if (ast.op === "&&" || ast.op === "||") {
    const flip = () => props.onChange({ ...ast, op: ast.op === "&&" ? "||" : "&&" });
    return (
      <span class="formula-builder-condition formula-builder-condition-group">
        <ConditionFace
          ast={ast.left}
          fields={props.fields}
          formulas={props.formulas}
          onChange={(left) => props.onChange({ ...ast, left })}
        />
        <button type="button" class="qb-op formula-builder-boolean-op" title="Toggle AND / OR" onClick={flip}>
          {ast.op === "&&" ? "AND" : "OR"}
        </button>
        <ConditionFace
          ast={ast.right}
          fields={props.fields}
          formulas={props.formulas}
          onChange={(right) => props.onChange({ ...ast, right })}
        />
      </span>
    );
  }
  return (
    <span class="formula-builder-condition">
      <ValueFace
        ast={ast.left}
        fields={props.fields}
        formulas={props.formulas}
        onChange={(left) => props.onChange({ ...ast, left })}
      />
      <OperatorSelect op={ast.op} onChange={(op) => props.onChange({ ...ast, op })} />
      <ValueFace
        ast={ast.right}
        fields={props.fields}
        formulas={props.formulas}
        onChange={(right) => props.onChange({ ...ast, right })}
      />
    </span>
  );
}

function OperatorSelect(props: { op: BinaryOp; onChange: (op: BinaryOp) => void }): JSX.Element {
  return (
    <select
      class="formula-builder-operator"
      value={props.op}
      title="Formula operator"
      onChange={(e) => props.onChange(e.currentTarget.value as BinaryOp)}
    >
      <For each={OPERATOR_OPTIONS}>
        {(option) => <option value={option.op}>{option.label}</option>}
      </For>
    </select>
  );
}

function ValueFace(props: {
  ast: Ast;
  fields: readonly string[];
  formulas: readonly [string, string][];
  onChange: (ast: Ast) => void;
}): JSX.Element {
  if (!isValueAst(props.ast)) {
    return <AstRawExpressionFace source={astToExpr(props.ast)} onAst={props.onChange} />;
  }

  const [open, setOpen] = createSignal(false);
  const commit = (next: Ast) => {
    props.onChange(next);
    setOpen(false);
  };

  return (
    <span class="qb-add-wrap formula-builder-value-wrap">
      <button
        type="button"
        class="qb-chip formula-builder-value-face"
        title="Edit formula value"
        onClick={(e) => {
          e.stopPropagation();
          setOpen(!open());
        }}
      >
        {valueLabel(props.ast)}
      </button>
      <Show when={open()}>
        <ValuePicker
          ast={props.ast}
          fields={props.fields}
          formulas={props.formulas}
          onCommit={commit}
        />
      </Show>
    </span>
  );
}

function ValuePicker(props: {
  ast: Ast;
  fields: readonly string[];
  formulas: readonly [string, string][];
  onCommit: (ast: Ast) => void;
}): JSX.Element {
  return (
    <div class="qb-picker formula-builder-picker" onClick={(e) => e.stopPropagation()}>
      <div class="qb-picker-title">Field</div>
      <For each={props.fields}>
        {(field) => (
          <button
            type="button"
            class="qb-menu-item formula-builder-pick-field"
            onClick={() => props.onCommit({ kind: "field", name: field })}
          >
            {field}
          </button>
        )}
      </For>
      <CommitTextInput
        placeholder="custom field"
        onCommit={(value) => props.onCommit({ kind: "field", name: value })}
      />
      <Show when={props.formulas.length > 0}>
        <div class="qb-divider" />
        <div class="qb-picker-title">Formula</div>
        <For each={props.formulas}>
          {([formula]) => (
            <button
              type="button"
              class="qb-menu-item formula-builder-pick-formula"
              onClick={() => props.onCommit({ kind: "formulaRef", name: formula })}
            >
              formula.{formula}
            </button>
          )}
        </For>
      </Show>
      <div class="qb-divider" />
      <div class="qb-picker-title">Literal</div>
      <CommitNumberInput onCommit={(value) => props.onCommit({ kind: "literal", value })} />
      <CommitTextInput
        placeholder="text literal"
        onCommit={(value) => props.onCommit({ kind: "literal", value })}
      />
      <div class="formula-builder-literal-row">
        <button type="button" class="qb-conn" onClick={() => props.onCommit({ kind: "literal", value: true })}>
          True
        </button>
        <button type="button" class="qb-conn" onClick={() => props.onCommit({ kind: "literal", value: false })}>
          False
        </button>
        <button type="button" class="qb-conn" onClick={() => props.onCommit({ kind: "literal", value: null })}>
          Empty
        </button>
      </div>
      <div class="qb-divider" />
      <div class="qb-picker-title">Function</div>
      <button
        type="button"
        class="qb-menu-item formula-builder-function"
        onClick={() => props.onCommit({ kind: "call", name: "if", args: [{ kind: "literal", value: true }, props.ast, { kind: "literal", value: null }] })}
      >
        if condition
      </button>
      <button
        type="button"
        class="qb-menu-item formula-builder-function"
        onClick={() => props.onCommit({ kind: "call", name: "isEmpty", args: [props.ast] })}
      >
        is empty
      </button>
      <button
        type="button"
        class="qb-menu-item formula-builder-function"
        onClick={() => props.onCommit({ kind: "call", name: "now", args: [] })}
      >
        now
      </button>
      <button
        type="button"
        class="qb-menu-item formula-builder-function"
        onClick={() => props.onCommit({ kind: "call", name: "today", args: [] })}
      >
        today
      </button>
      <div class="qb-divider" />
      <div class="qb-picker-title">Transform</div>
      <For each={TRANSFORMS}>
        {(spec) => <TransformPick object={props.ast} spec={spec} onCommit={props.onCommit} />}
      </For>
      <div class="qb-divider" />
      <div class="qb-picker-title">Raw expression</div>
      <RawCommitInput source={astToExpr(props.ast)} onCommit={props.onCommit} />
    </div>
  );
}

function CommitTextInput(props: { placeholder: string; onCommit: (value: string) => void }): JSX.Element {
  const [value, setValue] = createSignal("");
  const commit = () => {
    const text = value().trim();
    if (text) props.onCommit(text);
  };
  return (
    <input
      class="qb-input"
      placeholder={props.placeholder}
      value={value()}
      onInput={(e) => setValue(e.currentTarget.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          commit();
        }
      }}
    />
  );
}

function CommitNumberInput(props: { onCommit: (value: number) => void }): JSX.Element {
  const [value, setValue] = createSignal("");
  const parsed = () => Number(value());
  const ready = () => value().trim() !== "" && Number.isFinite(parsed());
  const commit = () => {
    if (ready()) props.onCommit(parsed());
  };
  return (
    <input
      class="qb-input"
      placeholder="number literal"
      value={value()}
      onInput={(e) => setValue(e.currentTarget.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          commit();
        }
      }}
    />
  );
}

function TransformPick(props: {
  object: Ast;
  spec: TransformSpec;
  onCommit: (ast: Ast) => void;
}): JSX.Element {
  const [editing, setEditing] = createSignal(false);
  const [values, setValues] = createSignal(props.spec.args.map((arg) => arg.value));
  const ready = () =>
    props.spec.args.every((arg, i) => arg.kind !== "number" || Number.isFinite(Number(values()[i] ?? "")));
  const commit = () => {
    if (!ready()) return;
    props.onCommit(transformAst(props.object, props.spec, values()));
  };
  const pick = () => {
    if (props.spec.args.length === 0) commit();
    else setEditing(true);
  };
  return (
    <Show
      when={editing()}
      fallback={
        <button
          type="button"
          class="qb-menu-item formula-builder-transform"
          onClick={pick}
        >
          {props.spec.label}
        </button>
      }
    >
      <div class="formula-builder-transform-args">
        <div class="formula-builder-transform-title">{props.spec.label}</div>
        <For each={props.spec.args}>
          {(arg, index) => (
            <input
              class="qb-input"
              placeholder={arg.placeholder}
              value={values()[index()]}
              onInput={(e) => {
                const next = [...values()];
                next[index()] = e.currentTarget.value;
                setValues(next);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  commit();
                }
              }}
            />
          )}
        </For>
        <button type="button" class="qb-commit" disabled={!ready()} onClick={commit}>
          Apply
        </button>
      </div>
    </Show>
  );
}

function RawCommitInput(props: { source: string; onCommit: (ast: Ast) => void }): JSX.Element {
  const [value, setValue] = createSignal(props.source);
  const [error, setError] = createSignal<string | null>(null);
  createEffect(() => {
    setValue(props.source);
    setError(null);
  });
  const commit = () => {
    const ast = parseAstText(value());
    if (!ast) {
      setError("Invalid expression");
      return;
    }
    props.onCommit(ast);
  };
  return (
    <div class="formula-builder-raw-commit">
      <input
        class="qb-input formula-builder-raw-input"
        classList={{ invalid: !!error() }}
        value={value()}
        onInput={(e) => setValue(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          }
        }}
        onBlur={commit}
      />
      <Show when={error()}>
        {(message) => <div class="formula-builder-raw-error">{message()}</div>}
      </Show>
    </div>
  );
}

function AstRawExpressionFace(props: { source: string; onAst: (ast: Ast) => void }): JSX.Element {
  const [value, setValue] = createSignal(props.source);
  const [error, setError] = createSignal<string | null>(null);
  createEffect(() => {
    setValue(props.source);
    setError(null);
  });
  const commit = () => {
    const ast = parseAstText(value());
    if (!ast) {
      setError("Invalid expression");
      return;
    }
    props.onAst(ast);
  };
  return (
    <span class="formula-builder-raw-face qb-chip-raw">
      <input
        class="formula-builder-raw-inline"
        classList={{ invalid: !!error() }}
        value={value()}
        onInput={(e) => setValue(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          }
        }}
        onBlur={commit}
      />
    </span>
  );
}

function RootRawExpressionFace(props: { source: string; onExpr: (expr: string) => void }): JSX.Element {
  return (
    <span class="formula-builder-raw-face formula-builder-root-raw qb-chip-raw">
      <input
        class="formula-builder-raw-inline"
        value={props.source}
        onInput={(e) => props.onExpr(e.currentTarget.value)}
      />
    </span>
  );
}

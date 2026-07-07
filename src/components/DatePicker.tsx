import { For, Show, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { datePicker, closeDatePicker, firstDayOfWeek, type DatePickerTarget } from "../ui";
import { readSchedule, setSchedule } from "../store";
import { fieldLabel, readField, writeField, type FieldId } from "../sheet/fields";

const MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
const DOW_BASE = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
// First day of week from the Tine display pref (see ui.firstDayOfWeek).
const startOfWeek = () => firstDayOfWeek();
const DOW = () => DOW_BASE.slice(startOfWeek()).concat(DOW_BASE.slice(0, startOfWeek()));
const pad2 = (n: number) => String(n).padStart(2, "0");
const fieldDate = (y: number, m: number, d: number) => `${y}-${pad2(m + 1)}-${pad2(d)}`;

// Calendar popup for SCHEDULED / DEADLINE. Writes `<yyyy-MM-dd EEE>` via the
// store; opening on an existing date pre-fills the shown month + selection.
export function DatePicker(): JSX.Element {
  return (
    <Show when={datePicker()}>
      {(dp) => <Picker bid={dp().blockId} which={dp().which} x={dp().x} y={dp().y} />}
    </Show>
  );
}

// Parse an org repeater cookie (`+1w`, `.+1w`, `++1w`) into UI state. `.+` means
// "from the completion day"; `+` from the stored date; `++` is catch-up. The mode
// is preserved verbatim so editing a `++` task's date doesn't rewrite it to `+`.
type RepMode = "+" | "++" | ".+";
function parseRepeater(r: string | null): { unit: string; num: number; mode: RepMode } {
  const m = r ? /^(\.\+|\+\+|\+)(\d+)([dwmy])$/.exec(r) : null;
  return m ? { unit: m[3], num: +m[2], mode: m[1] as RepMode } : { unit: "", num: 1, mode: "+" };
}

function isScheduleTarget(which: DatePickerTarget): which is "scheduled" | "deadline" {
  return which === "scheduled" || which === "deadline";
}

function parseIsoDate(value: string): { y: number; m: number; d: number; time: string | null } | null {
  const m = /^\s*(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2}))?\s*$/.exec(value);
  if (!m) return null;
  const y = Number(m[1]);
  const mo = Number(m[2]);
  const d = Number(m[3]);
  const hh = m[4] == null ? 0 : Number(m[4]);
  const mm = m[5] == null ? 0 : Number(m[5]);
  if (mo < 1 || mo > 12 || hh > 23 || mm > 59) return null;
  const dt = new Date(Date.UTC(y, mo - 1, d, hh, mm));
  if (dt.getUTCFullYear() !== y || dt.getUTCMonth() !== mo - 1 || dt.getUTCDate() !== d) return null;
  return { y, m: mo - 1, d, time: m[4] == null ? null : `${m[4]}:${m[5]}` };
}

function propDateSelection(bid: string, field: FieldId): { y: number; m: number; d: number; time: string | null } | null {
  const value = readField(bid, field)?.raw ?? "";
  return parseIsoDate(value);
}

function Picker(props: { bid: string; which: DatePickerTarget; x: number; y: number }): JSX.Element {
  const today = new Date();
  const scheduleSel = isScheduleTarget(props.which) ? readSchedule(props.bid, props.which) : null;
  const sel = scheduleSel ?? (isScheduleTarget(props.which) ? null : propDateSelection(props.bid, props.which.field));
  const [view, setView] = createSignal({
    y: sel?.y ?? today.getFullYear(),
    m: sel?.m ?? today.getMonth(),
  });

  // Recurrence: a unit ("" = no repeat), an interval N, and whether the next due
  // date counts from the completion day (`.+`) or the scheduled date (`+`).
  const init = parseRepeater(scheduleSel?.repeater ?? null);
  const [repUnit, setRepUnit] = createSignal(init.unit);
  const [repNum, setRepNum] = createSignal(init.num);
  const [repMode, setRepMode] = createSignal<RepMode>(init.mode);
  const repeater = (): string | null =>
    isScheduleTarget(props.which) && repUnit() ? `${repMode()}${Math.max(1, repNum())}${repUnit()}` : null;

  // Days laid out in weeks (leading blanks for the first-of-month offset).
  const grid = createMemo(() => {
    const { y, m } = view();
    // Leading blanks measured from the configured first day of week.
    const first = (new Date(y, m, 1).getDay() - startOfWeek() + 7) % 7;
    const days = new Date(y, m + 1, 0).getDate();
    const cells: (number | null)[] = [];
    for (let i = 0; i < first; i++) cells.push(null);
    for (let d = 1; d <= days; d++) cells.push(d);
    return cells;
  });

  const step = (delta: number) => {
    const total = view().y * 12 + view().m + delta;
    setView({ y: Math.floor(total / 12), m: ((total % 12) + 12) % 12 });
  };
  const writePickedDate = (y: number, m: number, d: number) => {
    const picked = fieldDate(y, m, d);
    if (isScheduleTarget(props.which)) {
      if (repeater()) setSchedule(props.bid, props.which, { y, m, d }, repeater());
      else writeField(props.bid, props.which, picked);
      return;
    }
    const time = props.which.fieldType === "datetime" ? propDateSelection(props.bid, props.which.field)?.time : null;
    writeField(props.bid, props.which.field, time ? `${picked} ${time}` : picked);
  };
  const pick = (d: number) => {
    writePickedDate(view().y, view().m, d);
    closeDatePicker();
  };
  const pickToday = () => {
    writePickedDate(today.getFullYear(), today.getMonth(), today.getDate());
    closeDatePicker();
  };
  const isToday = (d: number) =>
    view().y === today.getFullYear() && view().m === today.getMonth() && d === today.getDate();
  const isSel = (d: number) => !!sel && sel.y === view().y && sel.m === view().m && sel.d === d;
  const label = () => isScheduleTarget(props.which) ? props.which : fieldLabel(props.which.field);

  // Keep the popup on-screen. Reactive to the window size so it stays visible
  // even when the host window resizes after the picker opens — the quick-capture
  // window grows to make room for the picker, and this repositions it into view.
  const [winW, setWinW] = createSignal(typeof window !== "undefined" ? window.innerWidth : 1280);
  const [winH, setWinH] = createSignal(typeof window !== "undefined" ? window.innerHeight : 800);
  // Keep the popup on-screen: clamp its left edge so the full width (see
  // `.date-picker` = 284px in app.css, +margin) fits before the right window edge.
  const left = () => Math.max(4, Math.min(props.x, winW() - 300));
  const top = () => Math.max(4, Math.min(props.y, winH() - 300));
  onMount(() => {
    const onResize = () => {
      setWinW(window.innerWidth);
      setWinH(window.innerHeight);
    };
    // Esc closes the picker (capture phase so it beats the editor's own Esc — in
    // the quick-capture window a bubbling Esc would otherwise dismiss the whole
    // window and leave the picker open behind it).
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeDatePicker();
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
    <div class="dp-overlay" onClick={closeDatePicker} onContextMenu={(e) => { e.preventDefault(); closeDatePicker(); }}>
      <div class="date-picker" style={{ left: `${left()}px`, top: `${top()}px` }} onClick={(e) => e.stopPropagation()}>
        <div class="dp-head">
          <button class="dp-nav" onClick={() => step(-1)} title="Previous month">‹</button>
          <span class="dp-title">
            {MONTHS[view().m]} {view().y} <span class="dp-which">· {label()}</span>
          </span>
          <button class="dp-nav" onClick={() => step(1)} title="Next month">›</button>
        </div>
        <div class="dp-grid dp-dow">
          <For each={DOW()}>{(d) => <span class="dp-dowcell">{d}</span>}</For>
        </div>
        <div class="dp-grid">
          <For each={grid()}>
            {(d) => (
              <Show when={d !== null} fallback={<span class="dp-cell dp-empty" />}>
                <button
                  class="dp-cell"
                  classList={{ today: isToday(d!), selected: isSel(d!) }}
                  onClick={() => pick(d!)}
                >
                  {d}
                </button>
              </Show>
            )}
          </For>
        </div>
        <Show when={isScheduleTarget(props.which)}>
          <div class="dp-repeat" title="Pick a day to apply the repeat. On completion, a repeating task advances to its next date and reopens.">
            <select
              class="settings-select dp-rep-unit"
              value={repUnit()}
              onChange={(e) => setRepUnit(e.currentTarget.value)}
            >
              <option value="">No repeat</option>
              <option value="d">Daily</option>
              <option value="w">Weekly</option>
              <option value="m">Monthly</option>
              <option value="y">Yearly</option>
            </select>
            <Show when={repUnit()}>
              <span class="dp-rep-every">every</span>
              <input
                class="dp-rep-num"
                type="number"
                min="1"
                value={repNum()}
                onInput={(e) => setRepNum(Math.max(1, parseInt(e.currentTarget.value, 10) || 1))}
              />
              <label
                class="dp-rep-fromdone"
                title="Next due date is measured from the day you complete the task, not the scheduled date."
              >
                <input
                  type="checkbox"
                  checked={repMode() === ".+"}
                  onChange={(e) => setRepMode(e.currentTarget.checked ? ".+" : "+")}
                />
                from completion
              </label>
            </Show>
          </div>
        </Show>
        <div class="dp-foot">
          <button class="dp-btn" onClick={pickToday}>Today</button>
          <Show when={sel}>
            <button
              class="dp-btn dp-clear"
              onClick={() => {
                if (isScheduleTarget(props.which)) writeField(props.bid, props.which, "");
                else writeField(props.bid, props.which.field, "");
                closeDatePicker();
              }}
            >
              Clear
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
}

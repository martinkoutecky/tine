import { For, Show, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { datePicker, closeDatePicker, firstDayOfWeek } from "../ui";
import { readSchedule, setSchedule } from "../store";

const MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
const DOW_BASE = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
// First day of week from the Tine display pref (see ui.firstDayOfWeek).
const startOfWeek = () => firstDayOfWeek();
const DOW = () => DOW_BASE.slice(startOfWeek()).concat(DOW_BASE.slice(0, startOfWeek()));

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

function Picker(props: { bid: string; which: "scheduled" | "deadline"; x: number; y: number }): JSX.Element {
  const today = new Date();
  const sel = readSchedule(props.bid, props.which);
  const [view, setView] = createSignal({
    y: sel?.y ?? today.getFullYear(),
    m: sel?.m ?? today.getMonth(),
  });

  // Recurrence: a unit ("" = no repeat), an interval N, and whether the next due
  // date counts from the completion day (`.+`) or the scheduled date (`+`).
  const init = parseRepeater(sel?.repeater ?? null);
  const [repUnit, setRepUnit] = createSignal(init.unit);
  const [repNum, setRepNum] = createSignal(init.num);
  const [repMode, setRepMode] = createSignal<RepMode>(init.mode);
  const repeater = (): string | null =>
    repUnit() ? `${repMode()}${Math.max(1, repNum())}${repUnit()}` : null;

  // Optional clock time (`HH:mm`), like OG's "Add time". Seeded from the existing
  // timestamp so re-picking a date keeps the time (OG preserves it — GH #30). Kept
  // as independent state, applied on the same day-click that commits the date; a
  // native `<input type="time">` always yields 24h `HH:mm` regardless of locale.
  const [time, setTime] = createSignal<string | null>(sel?.time ?? null);
  // Default seed when "Add time" is first clicked: the current local time.
  const nowHHmm = () => {
    const n = new Date();
    return `${String(n.getHours()).padStart(2, "0")}:${String(n.getMinutes()).padStart(2, "0")}`;
  };

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
  const pick = (d: number) => {
    setSchedule(props.bid, props.which, { y: view().y, m: view().m, d }, repeater(), time());
    closeDatePicker();
  };
  const pickToday = () => {
    setSchedule(
      props.bid,
      props.which,
      { y: today.getFullYear(), m: today.getMonth(), d: today.getDate() },
      repeater(),
      time()
    );
    closeDatePicker();
  };
  const isToday = (d: number) =>
    view().y === today.getFullYear() && view().m === today.getMonth() && d === today.getDate();
  const isSel = (d: number) => !!sel && sel.y === view().y && sel.m === view().m && sel.d === d;

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
            {MONTHS[view().m]} {view().y} <span class="dp-which">· {props.which}</span>
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
        <div class="dp-time" title="Optional clock time. Pick a day to apply it.">
          <Show
            when={time() !== null}
            fallback={
              <button class="dp-addtime" onClick={() => setTime(nowHHmm())}>
                + Add time
              </button>
            }
          >
            <span class="dp-time-label">Time</span>
            <input
              class="dp-time-input"
              classList={{ invalid: !!time() && !/^\d{1,2}:\d{2}$/.test(time()!) }}
              type="text"
              inputmode="numeric"
              placeholder="HH:mm"
              maxlength="5"
              value={time()!}
              onInput={(e) => setTime(e.currentTarget.value || null)}
            />
            <button class="dp-time-clear" title="Remove time" onClick={() => setTime(null)}>
              ×
            </button>
          </Show>
        </div>
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
        <div class="dp-foot">
          <button class="dp-btn" onClick={pickToday}>Today</button>
          <Show when={sel}>
            <button
              class="dp-btn dp-clear"
              onClick={() => { setSchedule(props.bid, props.which, null); closeDatePicker(); }}
            >
              Clear
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
}

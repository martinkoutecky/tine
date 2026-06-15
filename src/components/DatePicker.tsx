import { For, Show, createMemo, createSignal, type JSX } from "solid-js";
import { datePicker, closeDatePicker } from "../ui";
import { readSchedule, setSchedule } from "../store";

const MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
const DOW = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

// Calendar popup for SCHEDULED / DEADLINE. Writes `<yyyy-MM-dd EEE>` via the
// store; opening on an existing date pre-fills the shown month + selection.
export function DatePicker(): JSX.Element {
  return (
    <Show when={datePicker()}>
      {(dp) => <Picker bid={dp().blockId} which={dp().which} x={dp().x} y={dp().y} />}
    </Show>
  );
}

function Picker(props: { bid: string; which: "scheduled" | "deadline"; x: number; y: number }): JSX.Element {
  const today = new Date();
  const sel = readSchedule(props.bid, props.which);
  const [view, setView] = createSignal({
    y: sel?.y ?? today.getFullYear(),
    m: sel?.m ?? today.getMonth(),
  });

  // Days laid out in weeks (leading blanks for the first-of-month offset).
  const grid = createMemo(() => {
    const { y, m } = view();
    const first = new Date(y, m, 1).getDay();
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
    setSchedule(props.bid, props.which, { y: view().y, m: view().m, d });
    closeDatePicker();
  };
  const pickToday = () => {
    setSchedule(props.bid, props.which, {
      y: today.getFullYear(),
      m: today.getMonth(),
      d: today.getDate(),
    });
    closeDatePicker();
  };
  const isToday = (d: number) =>
    view().y === today.getFullYear() && view().m === today.getMonth() && d === today.getDate();
  const isSel = (d: number) => !!sel && sel.y === view().y && sel.m === view().m && sel.d === d;

  // Keep the popup on-screen.
  const left = Math.min(props.x, (typeof window !== "undefined" ? window.innerWidth : 1280) - 260);
  const top = Math.min(props.y, (typeof window !== "undefined" ? window.innerHeight : 800) - 300);

  return (
    <div class="dp-overlay" onClick={closeDatePicker} onContextMenu={(e) => { e.preventDefault(); closeDatePicker(); }}>
      <div class="date-picker" style={{ left: `${left}px`, top: `${top}px` }} onClick={(e) => e.stopPropagation()}>
        <div class="dp-head">
          <button class="dp-nav" onClick={() => step(-1)} title="Previous month">‹</button>
          <span class="dp-title">
            {MONTHS[view().m]} {view().y} <span class="dp-which">· {props.which}</span>
          </span>
          <button class="dp-nav" onClick={() => step(1)} title="Next month">›</button>
        </div>
        <div class="dp-grid dp-dow">
          <For each={DOW}>{(d) => <span class="dp-dowcell">{d}</span>}</For>
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

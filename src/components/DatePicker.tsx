import { stepMonth } from "./primitives";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { datePicker, closeDatePicker, firstDayOfWeek, type DatePickerTarget } from "../ui";
import { readSchedule, setSchedule } from "../document";
import { fieldLabel, readField, writeFieldVisibly, type FieldId } from "../sheet/fields";
import { daysInCalendarMonth, parseIsoDateLike, utcCalendarMillis } from "../sheet/typed";
import { registerTransientLayer } from "../transientLayers";
import { captureBinding, stillBound, refuseStaleWrite } from "../binding";

import { parseRepeater, type RepMode } from "../editor/repeat";
import { appNow, journalTitle, localCalendarDate } from "../journal";
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

// Shared calendar: planning drafts and journal links commit on Done/outside/Enter;
// Escape cancels. Typed sheet day clicks also commit immediately. Existing planning/
// property dates seed the draft; journal links and empty dates start at today.
export function DatePicker(): JSX.Element {
  return (
    <Show when={datePicker()} keyed>
      {(dp) => <Picker bid={dp.blockId} which={dp.which} x={dp.x} y={dp.y} />}
    </Show>
  );
}

function isScheduleTarget(which: DatePickerTarget): which is "scheduled" | "deadline" {
  return which === "scheduled" || which === "deadline";
}

function propDateSelection(bid: string, field: FieldId): { y: number; m: number; d: number; time: string | null } | null {
  const value = readField(bid, field)?.raw ?? "";
  return parseIsoDateLike(value);
}

function Picker(props: { bid: string; which: DatePickerTarget; x: number; y: number }): JSX.Element {
  let root: HTMLDivElement | undefined;
  const binding = captureBinding();
  const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const close = () => {
    closeDatePicker();
    queueMicrotask(() => {
      if (stillBound(binding) && !datePicker() && opener?.isConnected) opener.focus();
    });
  };
  // I-20: writes and focus restoration belong to the opening graph.
  const bound = () => stillBound(binding) && datePicker() !== null || (refuseStaleWrite("The date"), false);
  createEffect(() => {
    const unregister = registerTransientLayer({
      id: "date-picker",
      root: () => root ?? null,
      trigger: () => stillBound(binding) ? opener : null,
      dismiss: () => { close(); return true; },
    });
    onCleanup(unregister);
  });
  const today = appNow();
  const scheduleSel = isScheduleTarget(props.which) ? readSchedule(props.bid, props.which) : null;
  const sel = scheduleSel ?? (isScheduleTarget(props.which) || "insertJournal" in props.which ? null : propDateSelection(props.bid, props.which.field));
  const [view, setView] = createSignal({
    y: sel?.y ?? today.getFullYear(),
    m: sel?.m ?? today.getMonth(),
  });

  const [cursor, setCursor] = createSignal({
    y: sel?.y ?? today.getFullYear(), m: sel?.m ?? today.getMonth(), d: sel?.d ?? today.getDate(),
  });
  const focusDay = () => root?.querySelector<HTMLButtonElement>(`[data-day="${cursor().d}"]`)?.focus();
  const moveDay = (delta: number) => {
    const current = cursor();
    const next = new Date(utcCalendarMillis(current.y, current.m, current.d + delta));
    const value = { y: next.getUTCFullYear(), m: next.getUTCMonth(), d: next.getUTCDate() };
    setCursor(value);
    setView({ y: value.y, m: value.m });
    queueMicrotask(focusDay);
  };

  // Recurrence: a unit ("" = no repeat), an interval N, and whether the next due
  // date counts from the completion day (`.+`) or the scheduled date (`+`).
  const init = parseRepeater(scheduleSel?.repeater ?? null);
  const [repUnit, setRepUnit] = createSignal(init.unit);
  const [repNum, setRepNum] = createSignal(init.num);
  const [repMode, setRepMode] = createSignal<RepMode>(init.mode);
  const repeater = (): string | null =>
    isScheduleTarget(props.which) && repUnit() ? `${repMode()}${Math.max(1, repNum())}${repUnit()}` : null;

  // Optional clock time (`HH:mm`), like OG's "Add time". Seeded from the existing
  // timestamp so the entire draft preserves it until commit (GH #30).
  const [time, setTime] = createSignal<string | null>(sel?.time ?? null);
  // Default seed when "Add time" is first clicked: the current local time.
  const nowHHmm = () => {
    const n = appNow();
    return `${String(n.getHours()).padStart(2, "0")}:${String(n.getMinutes()).padStart(2, "0")}`;
  };

  // Days laid out in weeks (leading blanks for the first-of-month offset).
  const grid = createMemo(() => {
    const { y, m } = view();
    // Leading blanks measured from the configured first day of week.
    // Shared calendar answerer: `new Date(y, …)` maps years 0–99 to 1900–1999.
    const first = (new Date(utcCalendarMillis(y, m, 1)).getUTCDay() - startOfWeek() + 7) % 7;
    const days = daysInCalendarMonth(y, m);
    const cells: (number | null)[] = [];
    for (let i = 0; i < first; i++) cells.push(null);
    for (let d = 1; d <= days; d++) cells.push(d);
    return cells;
  });

  const step = (delta: number) => {
    const next = stepMonth(view(), delta);
    setCursor({ ...next, d: Math.min(cursor().d, daysInCalendarMonth(next.y, next.m)) });
    setView(next);
  };
  const writePickedDate = (y: number, m: number, d: number) => {
    if (!bound()) return;
    const picked = fieldDate(y, m, d);
    if (isScheduleTarget(props.which)) {
      setSchedule(props.bid, props.which, { y, m, d }, repeater(), time());
      return;
    }
    if ("insertJournal" in props.which) {
      props.which.insertJournal(journalTitle(localCalendarDate(y, m, d)!));
      return;
    }
    const fieldTime = props.which.fieldType === "datetime" ? propDateSelection(props.bid, props.which.field)?.time : null;
    writeFieldVisibly(props.bid, props.which.field, fieldTime ? `${picked} ${fieldTime}` : picked);
  };
  const [chosen, setChosen] = createSignal({ ...cursor() });
  const commitDraft = () => {
    const date = chosen();
    writePickedDate(date.y, date.m, date.d);
    close();
  };
  const selectDate = (date: { y: number; m: number; d: number }) => {
    setChosen(date); setCursor(date); setView({ y: date.y, m: date.m });
    if (typeof props.which === "object" && "field" in props.which) commitDraft();
  };
  const pick = (d: number) => selectDate({ ...view(), d });
  const pickToday = () => selectDate({ y: today.getFullYear(), m: today.getMonth(), d: today.getDate() });
  const isToday = (d: number) =>
    view().y === today.getFullYear() && view().m === today.getMonth() && d === today.getDate();
  const isSel = (d: number) => chosen().y === view().y && chosen().m === view().m && chosen().d === d;
  const label = () => isScheduleTarget(props.which) ? props.which : "insertJournal" in props.which ? "journal" : fieldLabel(props.which.field);

  // Keep the popup on-screen. Reactive to the window size so it stays visible
  // even when the host window resizes after the picker opens — the quick-capture
  // window grows to make room for the picker, and this repositions it into view.
  const [winW, setWinW] = createSignal(typeof window !== "undefined" ? window.innerWidth : 1280);
  const [winH, setWinH] = createSignal(typeof window !== "undefined" ? window.innerHeight : 800);
  // Keep the popup on-screen: clamp its left edge so the full width (see
  // `.date-picker` = 284px in app.css, +margin) fits before the right window edge.
  const left = () => Math.max(4, Math.min(props.x, winW() - 300));
  const [height, setHeight] = createSignal(300);
  const top = () => Math.max(4, Math.min(props.y, winH() - height() - 4));
  onMount(() => {
    queueMicrotask(focusDay);
    const measure = () => setHeight(root?.getBoundingClientRect().height || 300);
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    if (root) observer?.observe(root);
    measure();
    const onResize = () => {
      setWinW(window.innerWidth);
      setWinH(window.innerHeight);
    };
    window.addEventListener("resize", onResize);
    onCleanup(() => {
      observer?.disconnect();
      window.removeEventListener("resize", onResize);
    });
  });

  return (
    <div class="dp-overlay" onClick={commitDraft} onContextMenu={(e) => { e.preventDefault(); commitDraft(); }}>
      <div ref={root} class="date-picker" style={{ left: `${left()}px`, top: `${top()}px` }} onClick={(e) => e.stopPropagation()}
        role="dialog" aria-label={`Choose ${label()} date`}
        onKeyDown={(e) => {
          if (e.isComposing || e.keyCode === 229) return;
          if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); return; }
          if (e.key === "Enter" && !(e.target as HTMLElement).matches("button:not(.dp-cell), select")) {
            e.preventDefault(); e.stopPropagation();
            if ((e.target as HTMLElement).matches(".dp-cell")) setChosen({ ...cursor() });
            commitDraft(); return;
          }
          if (!(e.target as HTMLElement).matches(".dp-cell")) return;
          const delta = ({ ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 } as Record<string, number>)[e.key];
          if (delta !== undefined) { e.preventDefault(); e.stopPropagation(); moveDay(delta); }
        }}>
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
                  data-day={d!}
                  tabIndex={cursor().y === view().y && cursor().m === view().m && cursor().d === d ? 0 : -1}
                  onFocus={() => setCursor({ ...view(), d: d! })}
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
          <div class="dp-time" title="Optional clock time. Done or clicking outside applies the date, time and repeat.">
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
          <div class="dp-repeat" title="Done or clicking outside applies the repeat. On completion, a repeating task advances to its next date and reopens.">
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
        <p class="dp-help">Done or click outside to apply. Escape cancels.</p>
        <div class="dp-foot">
          <button class="dp-btn" onClick={commitDraft}>Done</button>
          <button class="dp-btn" onClick={pickToday}>Today</button>
          <Show when={sel}>
            <button
              class="dp-btn dp-clear"
              onClick={() => {
                if (!bound()) return close();
                if (isScheduleTarget(props.which)) writeFieldVisibly(props.bid, props.which, "");
                else if ("field" in props.which) writeFieldVisibly(props.bid, props.which.field, "");
                close();
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

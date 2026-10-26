import { t } from "../i18n";
import { stepMonth } from "./primitives";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { datePicker, closeDatePicker, firstDayOfWeek, type DatePickerTarget } from "../ui";
import { readSchedule, setSchedule } from "../document";
import { fieldLabel, readField, writeFieldVisibly, type FieldId } from "../sheet/fields";
import { daysInCalendarMonth, parseIsoDateLike, utcCalendarMillis } from "../sheet/typed";
import { registerTransientLayer } from "../transientLayers";
import { captureBinding, bindingCurrent, refuseStaleWrite } from "../binding";

import { parseRepeater, type RepMode } from "../editor/repeat";
import { appNow, journalTitle, localCalendarDate } from "../journal";
import { isHTMLElementNode, newResizeObserver, useOwnerWindow } from "../windowRealm";
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
      {(dp) => <Picker bid={dp.blockId} which={dp.which} x={dp.x} y={dp.y} resolveWriteTarget={dp.resolveWriteTarget} />}
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

function Picker(props: { bid: string; which: DatePickerTarget; x: number; y: number; resolveWriteTarget?: () => string | null }): JSX.Element {
  let root: HTMLDivElement | undefined;
  const binding = captureBinding();
  const ownerWindow = useOwnerWindow();
  const focused = ownerWindow.document.activeElement;
  const opener = isHTMLElementNode(focused) ? focused : null;
  const close = () => {
    closeDatePicker();
    queueMicrotask(() => {
      if (bindingCurrent(binding) && !datePicker() && opener?.isConnected) opener.focus();
    });
  };
  // I-20: writes and focus restoration belong to the opening graph.
  const bound = () => bindingCurrent(binding) && datePicker() !== null || (refuseStaleWrite("The date"), false);
  createEffect(() => {
    const unregister = registerTransientLayer({
      id: "date-picker",
      root: () => root ?? null,
      trigger: () => bindingCurrent(binding) ? opener : null,
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
    if (!bound()) return false;
    const bid = props.resolveWriteTarget ? props.resolveWriteTarget() : props.bid;
    if (!bid) return false;
    const picked = fieldDate(y, m, d);
    if (isScheduleTarget(props.which)) {
      setSchedule(bid, props.which, { y, m, d }, repeater(), time());
      return true;
    }
    if ("insertJournal" in props.which) {
      props.which.insertJournal(journalTitle(localCalendarDate(y, m, d)!));
      return true;
    }
    const fieldTime = props.which.fieldType === "datetime" ? propDateSelection(bid, props.which.field)?.time : null;
    return writeFieldVisibly(bid, props.which.field, fieldTime ? `${picked} ${fieldTime}` : picked);
  };
  const [chosen, setChosen] = createSignal({ ...cursor() });
  const commitDraft = () => {
    const date = chosen();
    if (writePickedDate(date.y, date.m, date.d)) close();
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
  const label = () => isScheduleTarget(props.which) ? t("date-target-label", { target: props.which }) : "insertJournal" in props.which ? t("date-target-label", { target: "journal" }) : fieldLabel(props.which.field);

  // Keep the popup on-screen. Reactive to the window size so it stays visible
  // even when the host window resizes after the picker opens — the quick-capture
  // window grows to make room for the picker, and this repositions it into view.
  const [winW, setWinW] = createSignal(ownerWindow ? ownerWindow.innerWidth : 1280);
  const [winH, setWinH] = createSignal(ownerWindow ? ownerWindow.innerHeight : 800);
  // Keep the popup on-screen: clamp its left edge so the full width (see
  // `.date-picker` = 284px in app.css, +margin) fits before the right window edge.
  const left = () => Math.max(4, Math.min(props.x, winW() - 300));
  const [height, setHeight] = createSignal(300);
  const top = () => Math.max(4, Math.min(props.y, winH() - height() - 4));
  onMount(() => {
    queueMicrotask(focusDay);
    const measure = () => setHeight(root?.getBoundingClientRect().height || 300);
    const observer = root ? newResizeObserver(root, measure) : null;
    if (root) observer?.observe(root);
    measure();
    const onResize = () => {
      setWinW(ownerWindow.innerWidth);
      setWinH(ownerWindow.innerHeight);
    };
    ownerWindow.addEventListener("resize", onResize);
    onCleanup(() => {
      observer?.disconnect();
      ownerWindow.removeEventListener("resize", onResize);
    });
  });

  return (
    <div class="dp-overlay" onClick={commitDraft} onContextMenu={(e) => { e.preventDefault(); commitDraft(); }}>
      <div ref={root} class="date-picker" style={{ left: `${left()}px`, top: `${top()}px` }} onClick={(e) => e.stopPropagation()}
        role="dialog" aria-label={t("date-date-picker-choose-date", { value1: label() })}
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
          <button class="dp-nav" onClick={() => step(-1)} title={t("date-date-picker-previous-month")}>‹</button>
          <span class="dp-title">
            {MONTHS[view().m]} {view().y} <span class="dp-which">· {label()}</span>
          </span>
          <button class="dp-nav" onClick={() => step(1)} title={t("date-date-picker-next-month")}>›</button>
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
          <div class="dp-time" title={t("date-date-picker-optional-clock-time-done-or-clicking-outside-applies-the-date")}>
            <Show
              when={time() !== null}
              fallback={
                <button class="dp-addtime" onClick={() => setTime(nowHHmm())}>
                  {t("date-date-picker-add-time")}</button>
              }
            >
              <span class="dp-time-label">{t("date-date-picker-time")}</span>
              <input
                class="dp-time-input"
                classList={{ invalid: !!time() && !/^\d{1,2}:\d{2}$/.test(time()!) }}
                type="text"
                inputmode="numeric"
                placeholder={t("date-date-picker-hh-mm")}
                maxlength="5"
                value={time()!}
                onInput={(e) => setTime(e.currentTarget.value || null)}
              />
              <button class="dp-time-clear" title={t("date-date-picker-remove-time")} onClick={() => setTime(null)}>
                ×
              </button>
            </Show>
          </div>
          <div class="dp-repeat" title={t("date-date-picker-done-or-clicking-outside-applies-the-repeat-on-completion-a")}>
            <select
              class="settings-select dp-rep-unit"
              value={repUnit()}
              onChange={(e) => setRepUnit(e.currentTarget.value)}
            >
              <option value="">{t("date-date-picker-no-repeat")}</option>
              <option value="d">{t("date-date-picker-daily")}</option>
              <option value="w">{t("date-date-picker-weekly")}</option>
              <option value="m">{t("date-date-picker-monthly")}</option>
              <option value="y">{t("date-date-picker-yearly")}</option>
            </select>
            <Show when={repUnit()}>
              <span class="dp-rep-every">{t("date-date-picker-every")}</span>
              <input
                class="dp-rep-num"
                type="number"
                min="1"
                value={repNum()}
                onInput={(e) => setRepNum(Math.max(1, parseInt(e.currentTarget.value, 10) || 1))}
              />
              <label
                class="dp-rep-fromdone"
                title={t("date-date-picker-next-due-date-is-measured-from-the-day-you-complete")}
              >
                <input
                  type="checkbox"
                  checked={repMode() === ".+"}
                  onChange={(e) => setRepMode(e.currentTarget.checked ? ".+" : "+")}
                />
                {t("date-date-picker-from-completion")}</label>
            </Show>
          </div>
        </Show>
        <div class="dp-foot">
          <button class="dp-btn" onClick={commitDraft}>{t("date-date-picker-done")}</button>
          <button class="dp-btn" onClick={pickToday}>{t("date-date-picker-today")}</button>
          <Show when={sel}>
            <button
              class="dp-btn dp-clear"
              onClick={() => {
                if (!bound()) return;
                const bid = props.resolveWriteTarget ? props.resolveWriteTarget() : props.bid;
                if (!bid) return;
                if (isScheduleTarget(props.which) && !writeFieldVisibly(bid, props.which, "")) return;
                else if (typeof props.which === "object" && "field" in props.which && !writeFieldVisibly(bid, props.which.field, "")) return;
                close();
              }}
            >
              {t("date-date-picker-clear")}</button>
          </Show>
        </div>
      </div>
    </div>
  );
}

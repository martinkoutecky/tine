import { For, Show, createMemo, createResource, createSignal, type JSX } from "solid-js";
import { openPage } from "../router";
import { journalTitle } from "../journal";
import { graphMeta, dataRev } from "../ui";
import { backend } from "../backend";

const MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
const DOW_BASE = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

// Topbar calendar button → a month-grid popup to jump to any date's journal.
// Custom popup (not a native date input) so it closes on outside-click and
// honours :start-of-week, matching the scheduled/deadline picker.
export function CalendarJump(): JSX.Element {
  const [open, setOpen] = createSignal(false);
  const today = new Date();
  const [view, setView] = createSignal({ y: today.getFullYear(), m: today.getMonth() });
  const sow = () => {
    const n = graphMeta()?.start_of_week ?? 0;
    return n >= 0 && n <= 6 ? n : 0;
  };
  const dow = () => DOW_BASE.slice(sow()).concat(DOW_BASE.slice(0, sow()));

  const grid = createMemo(() => {
    const { y, m } = view();
    const first = (new Date(y, m, 1).getDay() - sow() + 7) % 7;
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
    openPage(journalTitle(new Date(view().y, view().m, d)), "journal");
    setOpen(false);
  };
  const isToday = (d: number) =>
    view().y === today.getFullYear() && view().m === today.getMonth() && d === today.getDate();

  // Journal days that have content, fetched while the popup is open (re-fetched
  // on dataRev so adding content updates the dots). yyyymmdd keys, month 1-based.
  const [contentDays] = createResource(
    () => (open() ? dataRev() : null),
    () => backend().journalContentDays()
  );
  const haveContent = createMemo(() => new Set(contentDays() ?? []));
  const hasContent = (d: number) =>
    haveContent().has(view().y * 10000 + (view().m + 1) * 100 + d);

  return (
    <>
      <button class="icon-btn" title="Go to date" onClick={() => setOpen(!open())}>
        <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
          <rect x="4" y="5" width="16" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.7" />
          <line x1="4" y1="9.5" x2="20" y2="9.5" stroke="currentColor" stroke-width="1.7" />
          <line x1="8.5" y1="3" x2="8.5" y2="7" stroke="currentColor" stroke-width="1.7" />
          <line x1="15.5" y1="3" x2="15.5" y2="7" stroke="currentColor" stroke-width="1.7" />
        </svg>
      </button>
      <Show when={open()}>
        <div class="dp-overlay" onClick={() => setOpen(false)}>
          <div class="date-picker calendar-jump-pop" onClick={(e) => e.stopPropagation()}>
            <div class="dp-head">
              <button class="dp-nav" onClick={() => step(-1)} title="Previous month">‹</button>
              <span class="dp-title">{MONTHS[view().m]} {view().y}</span>
              <button class="dp-nav" onClick={() => step(1)} title="Next month">›</button>
            </div>
            <div class="dp-grid dp-dow">
              <For each={dow()}>{(d) => <span class="dp-dowcell">{d}</span>}</For>
            </div>
            <div class="dp-grid">
              <For each={grid()}>
                {(d) => (
                  <Show when={d !== null} fallback={<span class="dp-cell dp-empty" />}>
                    <button
                      class="dp-cell"
                      classList={{ today: isToday(d!), "has-content": hasContent(d!) }}
                      title={hasContent(d!) ? "Has an entry" : "Empty"}
                      onClick={() => pick(d!)}
                    >
                      {d}
                    </button>
                  </Show>
                )}
              </For>
            </div>
            <div class="dp-foot">
              <button class="dp-today" onClick={() => pick(today.getDate())}>Today</button>
            </div>
          </div>
        </div>
      </Show>
    </>
  );
}

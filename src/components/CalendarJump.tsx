import { type JSX } from "solid-js";
import { openPage } from "../router";
import { journalTitle } from "../journal";

// Topbar calendar button: pick any date and jump to its journal page. Uses a
// native date input (overlaid, transparent) for a zero-dependency month picker.
export function CalendarJump(): JSX.Element {
  return (
    <label class="icon-btn calendar-jump" title="Go to date">
      <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
        <rect x="4" y="5" width="16" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.7" />
        <line x1="4" y1="9.5" x2="20" y2="9.5" stroke="currentColor" stroke-width="1.7" />
        <line x1="8.5" y1="3" x2="8.5" y2="7" stroke="currentColor" stroke-width="1.7" />
        <line x1="15.5" y1="3" x2="15.5" y2="7" stroke="currentColor" stroke-width="1.7" />
      </svg>
      <input
        type="date"
        class="calendar-jump-input"
        onChange={(e) => {
          const v = e.currentTarget.value; // yyyy-mm-dd
          if (!v) return;
          const [y, m, d] = v.split("-").map(Number);
          openPage(journalTitle(new Date(y, m - 1, d)), "journal");
          e.currentTarget.value = "";
        }}
      />
    </label>
  );
}

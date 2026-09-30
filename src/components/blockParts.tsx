/** Small stateless pieces of the block view and editor, split out of Block.tsx:
 * task-marker/checkbox toggles on one block (each one `setRaw` of that block),
 * the clock badge and calendar glyph, and pure text/DOM helpers the editor uses
 * (in-block list line detection, first visible line, nearest vertical scroller,
 * HH:MM stamp). None of them reads or writes anything beyond its arguments and,
 * for the two toggles, the block they name. */
import { For, type JSX } from "solid-js";
import { node as docNode, pageByName, setRaw } from "../document";
import { toggleMarkerLabel, toggleTaskDone } from "../editor/repeat";
import type { LogbookInfo } from "../logbook";
import { logbookWithSecondSupport, timetrackingEnabled, workflow } from "../ui";

import { appNow } from "../journal";
// Marker-label clicks follow OG's separate two-state toggle (TODO <-> DOING,
// LATER <-> NOW). Keyboard marker cycling remains cycleMarkerSmart and may
// still reach DONE / no marker; a label click never removes completion.
export function toggleBlockMarkerLabel(id: string) {
  const raw = toggleMarkerLabel(docNode(id).raw, {
    format: formatForBlockId(id),
    enabled: timetrackingEnabled(),
    withSeconds: logbookWithSecondSupport(),
  });
  if (raw === null) return;
  setRaw(id, raw, { timetracking: false });
}

// Toggle the task checkbox (OG check/uncheck): open → DONE (rolling a repeater
// forward instead), DONE → the workflow's open marker. Used by the block checkbox.
export function toggleBlockCheckbox(id: string) {
  const raw = toggleTaskDone(docNode(id).raw, workflow(), formatForBlockId(id), {
    format: formatForBlockId(id),
    enabled: timetrackingEnabled(),
    withSeconds: logbookWithSecondSupport(),
  });
  if (raw !== null) setRaw(id, raw, { timetracking: false });
}

export function formatForBlockId(id: string): "md" | "org" {
  return pageByName(docNode(id)?.page)?.format ?? "md";
}

export function ClockBadge(props: { info: LogbookInfo }): JSX.Element {
  const rows = () => props.info.rows.slice().reverse().slice(0, 10);
  return (
    <span class="clock-badge" tabIndex={0}>
      <span class="clock-badge-label">{props.info.summary}</span>
      <span class="clock-tooltip" role="tooltip">
        <table>
          <thead>
            <tr>
              <th>Type</th>
              <th>Start</th>
              <th>End</th>
              <th>Span</th>
            </tr>
          </thead>
          <tbody>
            <For each={rows()}>
              {(r) => (
                <tr>
                  <td>{r.type}</td>
                  <td>{r.start}</td>
                  <td>{r.end ?? ""}</td>
                  <td>{r.span ?? ""}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </span>
    </span>
  );
}

/** Nearest ancestor that actually scrolls vertically — used to pin the scroll
 *  position across a textarea autosize measure (WebKitGTK reveals the caret on
 *  the transient height:auto collapse, jumping tall blocks to the bottom). */
export function nearestScrollableY(el: HTMLElement): HTMLElement | null {
  let n: HTMLElement | null = el.parentElement;
  while (n) {
    if (n.scrollHeight > n.clientHeight) {
      const oy = getComputedStyle(n).overflowY;
      if (oy === "auto" || oy === "scroll" || oy === "overlay") return n;
    }
    n = n.parentElement;
  }
  return null;
}

/** First visible (non-`key:: value`) line of a block's raw markdown — what the
 *  block-reference picker shows as the candidate's label. */
export function blockFirstLine(raw: string): string {
  for (const line of raw.split("\n")) {
    if (!/^\s*[\w-]+:: /.test(line) && line.trim() !== "") return line.trim();
  }
  return "";
}

/** If the caret sits on an in-block markdown list line (`+`/`*`/ordered — NOT the
 *  outline bullet `-`), return its parts, for caret-context list editing. */
// In-block list markers differ by format (see body.tsx): Markdown uses `+`/`*`
// (a leading `-` is the outline bullet), Org uses `-`/`+` (a leading `*` is a
// headline). Numbered works in both.
const LIST_LINE_MD = /^(\s*)([+*]|\d+[.)])(\s+)(\[[ xX]\]\s+)?/;
const LIST_LINE_ORG = /^(\s*)([-+]|\d+[.)])(\s+)(\[[ xX]\]\s+)?/;
export function listLineAt(
  text: string,
  caret: number,
  format: "md" | "org" = "md",
): { indent: string; marker: string; hasCheckbox: boolean; lineStart: number; prefixLen: number } | null {
  const lineStart = text.lastIndexOf("\n", caret - 1) + 1;
  let lineEnd = text.indexOf("\n", caret);
  if (lineEnd === -1) lineEnd = text.length;
  const re = format === "org" ? LIST_LINE_ORG : LIST_LINE_MD;
  const m = re.exec(text.slice(lineStart, lineEnd));
  if (!m) return null;
  return { indent: m[1], marker: m[2], hasCheckbox: !!m[4], lineStart, prefixLen: m[0].length };
}

// Small calendar glyph for date chips (SVG, not emoji — emoji tofu on WebKitGTK).
export function CalGlyph(): JSX.Element {
  return (
    <svg class="chip-cal" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="4" y="5" width="16" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="2" />
      <line x1="4" y1="9.5" x2="20" y2="9.5" stroke="currentColor" stroke-width="2" />
    </svg>
  );
}

export function timeStamp(d = appNow()): string {
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}
// Template support: session-cached list of templates, dynamic-var substitution,

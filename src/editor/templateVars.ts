// Logseq dynamic template variables. A template block's `<% … %>` placeholders
// are expanded when the template is inserted (NOT on normal render). Single
// source of truth for both insertion paths: the slash-command template insert
// (Block.tsx) and the default journal template (graph.ts).

import { journalTitle } from "../journal";
import { resolveDateToken } from "./dateExpr";

type NaturalDateParser = (text: string, reference?: Date) => Date | null;
let parseEnglishDate: NaturalDateParser | null = null;
let parserLoad: Promise<void> | null = null;

/** Prepare OG's natural-language date parser before expanding a template.
 * Chrono is deliberately a lazy chunk: ordinary app startup never needs its
 * ~98 kB emitted parser chunk. Both production template insertion paths await this
 * once before calling the otherwise synchronous recursive expander. */
export function prepareTemplateVars(): Promise<void> {
  if (parseEnglishDate) return Promise.resolve();
  if (!parserLoad) {
    parserLoad = import("chrono-node/dist/locales/en").then((chrono) => {
      parseEnglishDate = chrono.parseDate;
    });
  }
  return parserLoad;
}

/** The variables we advertise — as `/Template var: …` slash commands and in the
 *  "Make a template" hint. `<% date: <token> %>` (a date token like `today`,
 *  `+3d`, `2026-07-01`) is offered separately since it takes an argument. */
export const TEMPLATE_VARS: { label: string; insert: string }[] = [
  { label: "today", insert: "<% today %>" },
  { label: "yesterday", insert: "<% yesterday %>" },
  { label: "tomorrow", insert: "<% tomorrow %>" },
  { label: "current page", insert: "<% current page %>" },
  { label: "time", insert: "<% time %>" },
];

function clock(d: Date): string {
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** Replace dynamic template vars in a block's raw text. Unknown or unresolvable
 *  `<% … %>` are left verbatim (never dropped). `currentPage` is the page the
 *  template is being inserted into — used for `<% current page %>`. */
export function applyTemplateVars(raw: string, currentPage?: string): string {
  return raw.replace(/<%\s*([^%]*?)\s*%>/g, (whole, body) => {
    const kw = String(body).trim();
    const k = kw.toLowerCase();
    const d = new Date();
    if (k === "time" || k === "current time") return clock(d);
    if (k === "current page") return currentPage ? `[[${currentPage}]]` : whole;
    if (k === "today") return `[[${journalTitle(d)}]]`;
    if (k === "yesterday") {
      d.setDate(d.getDate() - 1);
      return `[[${journalTitle(d)}]]`;
    }
    if (k === "tomorrow") {
      d.setDate(d.getDate() + 1);
      return `[[${journalTitle(d)}]]`;
    }
    // <% date: <token> %> — resolve via the same date-token parser the query
    // builder uses (today / ±Nd / yyyy-MM-dd / journal title).
    const dm = /^date:\s*(.+)$/i.exec(kw);
    if (dm) {
      const resolved = resolveDateToken(dm[1].trim());
      return resolved ? `[[${journalTitle(resolved)}]]` : whole;
    }
    // Match Logseq's final parser path for any remaining expression. The Date
    // reference preserves Chrono's local-clock semantics; its result is then
    // formatted through the graph-configured journal title as usual.
    const resolved = kw && parseEnglishDate ? parseEnglishDate(kw, d) : null;
    return resolved ? `[[${journalTitle(resolved)}]]` : whole;
  });
}

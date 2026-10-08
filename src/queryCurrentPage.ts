// The page an advanced query's typed `:current-page` input binds to, and
// whether a query declares that input. OG `query_react.cljs` `resolve-input`
// binds `:current-page-fn` to `(or (state/get-current-page) (:page
// (state/get-default-home)) (date/today))`: the routed page of the focused
// pane, else the configured home page, else today's journal. It is NOT the
// page the query block renders on (og 14 Q2 follow-up; master binds the
// focused pane's page the same way).
import { configuredHomePage } from "./homePage";
import { journalTitle, appNow } from "./journal";
import { focusedRouter } from "./panes";

/** OG's `current-page-fn`: focused pane's routed page → default home → today. */
export function queryCurrentPage(): string {
  const route = focusedRouter().route();
  if (route.kind === "page" && route.name.trim()) return route.name;
  return configuredHomePage() ?? journalTitle(appNow());
}

// Recognize the typed Logseq input without treating an example in a string or
// `;;` comment as live. Only a direct token in the :inputs vector makes query
// execution depend on focused-pane navigation. (Master's recognizer, verbatim.)
export function declaresCurrentPageInput(source: string): boolean {
  const boundary = (ch: string | undefined) =>
    ch === undefined || /[\s,\[\](){}]/.test(ch);
  let i = 0;
  while (i < source.length) {
    if (source[i] === '"') {
      i += 1;
      while (i < source.length) {
        if (source[i] === "\\") i += 2;
        else if (source[i] === '"') {
          i += 1;
          break;
        } else i += 1;
      }
      continue;
    }
    if (source[i] === ";") {
      while (i < source.length && source[i] !== "\n") i += 1;
      continue;
    }
    if (
      source.startsWith(":inputs", i) &&
      boundary(source[i - 1]) &&
      boundary(source[i + ":inputs".length])
    ) {
      let cursor = i + ":inputs".length;
      while (cursor < source.length && /[\s,]/.test(source[cursor])) cursor += 1;
      if (source[cursor] !== "[") return false;
      let depth = 1;
      cursor += 1;
      while (cursor < source.length && depth > 0) {
        if (source[cursor] === '"') {
          cursor += 1;
          while (cursor < source.length) {
            if (source[cursor] === "\\") cursor += 2;
            else if (source[cursor] === '"') {
              cursor += 1;
              break;
            } else cursor += 1;
          }
          continue;
        }
        if (source[cursor] === ";") {
          while (cursor < source.length && source[cursor] !== "\n") cursor += 1;
          continue;
        }
        if ("[({".includes(source[cursor])) depth += 1;
        else if ("])}".includes(source[cursor])) depth -= 1;
        else if (
          depth === 1 &&
          source.slice(cursor, cursor + ":current-page".length).toLowerCase() ===
            ":current-page" &&
          boundary(source[cursor - 1]) &&
          boundary(source[cursor + ":current-page".length])
        ) {
          return true;
        }
        cursor += 1;
      }
      return false;
    }
    i += 1;
  }
  return false;
}

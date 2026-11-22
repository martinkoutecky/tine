// The page an advanced query's typed `:current-page` input binds to.
// OG `query_react.cljs` `resolve-input`
// binds `:current-page-fn` to `(or (state/get-current-page) (:page
// (state/get-default-home)) (date/today))`: the routed page of the focused
// pane, else the configured home page, else today's journal. It is NOT the
// page the query block renders on (og 14 Q2 follow-up; master binds the
// focused pane's page the same way).
import { configuredHomePage } from "./homePage";
import { journalTitle, currentDayKey, localDateFromDayKey } from "./journal";
import { focusedRouter } from "./panes";

/** OG's `current-page-fn`: focused pane's routed page → default home → today.
 *  "Today" is the reactive day, so a query bound to it moves to the new
 *  journal when the date changes (card hB3Lv8). */
export function queryCurrentPage(): string {
  const route = focusedRouter().route();
  if (route.kind === "page" && route.name.trim()) return route.name;
  return configuredHomePage() ?? journalTitle(localDateFromDayKey(currentDayKey()));
}

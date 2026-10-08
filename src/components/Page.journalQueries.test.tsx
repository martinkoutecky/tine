import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PageView } from "./Page";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { resetTabsToJournals } from "../router";
import { currentDayKey, journalTitle, appNow } from "../journal";
import { setGraphMeta } from "../graphSession";
import { agendaQuery, agendaDaysAhead, setAgendaDaysAhead } from "../ui";
import type { GraphMeta, PageRead } from "../types";
import { backendReadsQueries, blockRunResult } from "../tests/queryReadingsTestkit";
import { resetSharedQueryResultsForTests } from "../queryResultCache";

beforeAll(initParser);
afterEach(() => { resetStore(); resetTabsToJournals(); setGraphMeta(null); resetSharedQueryResultsForTests(); vi.restoreAllMocks(); document.body.replaceChildren(); });

it("renders independent configured queries before today's agenda and applies a live config edit", async () => {
  const today = journalTitle(appNow());
  const pages: PageRead[] = [today, "Sep 20th, 2026"].map((name, i) => ({
    id: `journals/jq-${i}.md`, name, title: name, kind: "journal", pre_block: null,
    blocks: [{ id: `jq-${i}`, raw: `Body ${i}`, collapsed: false, children: [] }],
  }));
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages, as_of_day: currentDayKey(), next_before_day: null, done: true });
  const meta = { scheduled_future_days: 12, default_journal_queries: [
    { title: "First", body: "query (task TODO)", error: null },
    { title: "Broken", body: "", error: "Invalid journal query" },
    { title: "Last", body: "query (task DOING)", error: null },
  ], journal_config_diagnostics: ["Invalid config value"] } as unknown as GraphMeta;
  setGraphMeta(meta);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.querySelectorAll(".default-journal-query")).toHaveLength(3));
    const days = host.querySelectorAll(".page-section");
    expect(days[1].querySelector(".default-journal-query")).toBeNull();
    expect(host.textContent).toContain("Invalid journal query");
    expect(host.textContent).toContain("Invalid config value");
    expect(host.querySelector(".today-queries")!.compareDocumentPosition(host.querySelector(".agenda-block")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(agendaQuery()).toContain("+12d");
    setGraphMeta({ ...meta, disable_scheduled_and_deadline_query: true, scheduled_future_days: 0, default_journal_queries: [] });
    await vi.waitFor(() => expect(host.querySelector(".agenda-block")).toBeNull());
    expect(host.querySelector(".default-journal-query")).toBeNull();
    expect(agendaQuery()).toContain("+0d");
  } finally { dispose(); }
});

it("keeps supported simple and advanced results usable beside an unsupported query, with collapse", async () => {
  const today = journalTitle(appNow());
  const advanced = '{:query [:find (pull ?b [*]) :where [?b :block/marker "TODO"]] :inputs [] :collapsed? true}';
  const unsupported = '{:query [:find ?b :where (graph-code ?b)]}';
  backendReadsQueries({
    "(task TODO)": { form: "(task TODO)" },
    [advanced]: { form: advanced, kind: "advanced", opts: "{:collapsed? true}" },
    [unsupported]: { form: unsupported, kind: "advanced" },
  });
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages: [{
    id: "journals/today.md", name: today, title: today, kind: "journal", pre_block: null, blocks: [],
  }], as_of_day: currentDayKey(), next_before_day: null, done: true });
  const run = vi.spyOn(backend(), "queryRun").mockImplementation(async query => {
    const source = "original" in query.source ? query.source.original : "";
    if (source === unsupported) return blockRunResult([], { supported: false, ignored: ["graph-code"] });
    return blockRunResult([{ page: "Tasks", kind: "page", blocks: [{
      id: source === advanced ? "advanced-hit" : "simple-hit",
      raw: source === advanced ? "TODO Advanced result" : "TODO Simple result",
      collapsed: false, children: [],
    }] }]);
  });
  setGraphMeta({ disable_scheduled_and_deadline_query: true, default_journal_queries: [
    { title: "Simple", body: "query (task TODO)", error: null },
    { title: "Unsupported", body: `query ${unsupported}`, error: null },
    { title: "Advanced", body: `query ${advanced}`, error: null },
  ] } as GraphMeta);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.textContent).toContain("Simple result"));
    await vi.waitFor(() => expect(host.textContent).toContain("query not run"));
    const regions = host.querySelectorAll(".default-journal-query");
    expect(regions[2].textContent).toContain("Advanced");
    expect(regions[2].querySelector(".query-collapse.collapsed")).not.toBeNull();
    expect(regions[2].textContent).not.toContain("Advanced result");
    (regions[2].querySelector(".query-collapse") as HTMLElement).click();
    await vi.waitFor(() => expect(regions[2].textContent).toContain("Advanced result"));
    expect(run.mock.calls.every(([, , context]) => context?.current_page === today)).toBe(true);
    expect(host.textContent).toContain("Simple result");
  } finally { dispose(); }
});

it("leaves the device's days-ahead setting in charge when config.edn does not set :scheduled/future-days", () => {
  const before = agendaDaysAhead();
  try {
    setAgendaDaysAhead(21);
    setGraphMeta({ scheduled_future_days: null, default_journal_queries: [] } as unknown as GraphMeta);
    expect(agendaQuery()).toContain("+21d");
    setGraphMeta({ scheduled_future_days: 3, default_journal_queries: [] } as unknown as GraphMeta);
    expect(agendaQuery()).toContain("+3d");
  } finally { setAgendaDaysAhead(before); }
});

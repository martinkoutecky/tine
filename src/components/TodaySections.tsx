// What shows under today's journal (OG page/today-queries, GH #605): the
// sections page when one is marked `tine/journal-sections:: true`, else
// config.edn `:default-queries` followed by the built-in agenda.
// Policy: src/journalSections.ts.
import { For, Match, Show, Switch, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { bindingCurrent, captureBinding } from "../binding";
import { dataRev, graphEpoch, graphMeta } from "../graphSession";
import { bindingOwner, graphOwner, readOwned } from "../owned";
import { errorFamily } from "../errorFamily";
import { macroNameForDialect } from "../editor/queryMacroName";
import { readOr } from "../resourceRead";
import { agendaQuery, agendaTql, openPageInSidebar } from "../ui";
import { ensureGuidePagesLoaded, guidePageName } from "../guide";
import { openPage } from "../router";
import { createPage } from "../document";
import { pushToast } from "../toasts";
import { t } from "../i18n";
import { QueryMacro } from "./Macro";
import { BeginQuery, inspectBeginQuery } from "./BeginQuery";
import { detectMacro } from "./Rendered";
import { ResourceFailure } from "./ResourceFailure";
import { DEFAULT_JOURNAL_SECTIONS_PAGE, locateSectionsPages, sectionItems, seedSectionsPage, type SectionItem } from "../journalSections";
import type { Format } from "../types";
import { soleBlockMacro } from "../render/parse";

const classify = (raw: string, format: Format): SectionItem["kind"] | null => {
  const macro = soleBlockMacro(raw, format);
  if (macro?.name === "upcoming" && macro.args.length === 0) return "upcoming";
  return detectMacro(raw, format)?.kind === "query" ? "query" : inspectBeginQuery(raw, format) ? "begin-query" : null;
};

type Sections = { page: string; items: SectionItem[]; ambiguous: string[] } | null;

export function JournalSectionsHeader(): JSX.Element {
  const openGuide = async () => {
    const owner = graphOwner();
    try {
      await ensureGuidePagesLoaded();
      if (owner()) openPageInSidebar(guidePageName("Reference/Journals, tasks, and scheduling"));
    } catch (error) {
      if (owner()) pushToast(t("guide-couldn-t-open-the-guide", { error: String(error) }), "error");
    }
  };
  return <div class="journal-sections-header">
    <span>{t("journal-sections-explanation")}</span>{" "}
    <button type="button" onClick={() => void openGuide()}>{t("help-guide")}</button>
  </div>;
}

async function readSections(): Promise<Sections> {
  const owner = graphOwner();
  const located = await readOwned(owner, locateSectionsPages());
  if (located.kind !== "current" || located.value.length === 0) return null;
  const names = [...located.value].sort((a, b) => a.localeCompare(b));
  const read = await readOwned(owner, backend().getPage(names[0], "page"));
  if (read.kind !== "current" || !read.value) return null;
  const format = read.value.format ?? "md";
  return { page: read.value.name, items: sectionItems(read.value.blocks, format, classify), ambiguous: names.slice(1) };
}

/** Create the sections page from the current config sections and open it.
 *  A page of the default name the user already has is never overwritten:
 *  the next free "Journal sections N" is taken (createPage refuses an existing
 *  file without a base revision). */
export async function customizeTodaySections(): Promise<void> {
  const binding = captureBinding();
  const owner = bindingOwner();
  const format: Format = graphMeta()?.preferred_format ?? "md";
  const agenda = `{{${macroNameForDialect(false)} ${agendaTql()}}}`;
  for (let n = 1; n < 100; n += 1) {
    const name = n === 1 ? DEFAULT_JOURNAL_SECTIONS_PAGE : `${DEFAULT_JOURNAL_SECTIONS_PAGE} ${n}`;
    try {
      const existing = await readOwned(owner, backend().getPage(name, "page"));
      if (existing.kind === "stale") return;
      if (existing.value) continue;
      await createPage(name, seedSectionsPage(name, format, agenda), { bindingGeneration: binding.backendGeneration });
    } catch (error) {
      if (bindingCurrent(binding) && errorFamily(error) === "conflict") continue;
      pushToast(t("journal-sections-create-failed", { page: name, error: String(error) }), "error");
      return;
    }
    if (bindingCurrent(binding)) openPage(name);
    return;
  }
}

export function TodaySections(props: { today: string; upcoming?: JSX.Element }): JSX.Element {
  const [sections, { refetch }] = createResource(() => `${graphEpoch()}\0${dataRev()}`, readSections);
  const current = () => readOr(sections, null, "today sections");
  const [busy, setBusy] = createSignal(false);
  const customize = () => {
    if (busy()) return;
    setBusy(true);
    void customizeTodaySections().finally(() => setBusy(false));
  };
  return (
    <>
      <ResourceFailure of={sections} what="today's sections" onRetry={() => void refetch()} />
      <Show when={current()} fallback={
        // Resolve which sections own this journal before showing the fallback;
        // otherwise config results can flash before a sections page replaces them.
        // A settled fallback stays mounted through ordinary save refetches.
        <Show when={sections.state === "ready" || sections.state === "refreshing"}>
          <ConfigSections today={props.today} />
          <button class="today-sections-customize" type="button" disabled={busy()} onClick={customize}>
            {t("journal-sections-customize")}
          </button>
        </Show>
      }>
        {(found) => (
          <div class="today-queries today-sections" data-sections-page={found().page}>
            <Show when={found().ambiguous.length > 0}>
              <div class="config-diagnostic" role="status">
                {t("journal-sections-ambiguous", { page: found().page, others: found().ambiguous.join(", ") })}
              </div>
            </Show>
            <For each={found().items}>
              {(item) => <div class="default-journal-query">
                <Switch>
                  <Match when={item.kind === "upcoming"}>{props.upcoming}</Match>
                  <Match when={item.kind === "query" && detectMacro(item.raw, item.format)}>
                    {(macro) => <QueryMacro body={macro().inner} currentPage={props.today} hideWhenEmpty />}
                  </Match>
                  <Match when={item.kind === "begin-query" && inspectBeginQuery(item.raw, item.format)}>
                    {(match) => <BeginQuery match={match()} currentPage={props.today} hideWhenEmpty />}
                  </Match>
                </Switch>
              </div>}
            </For>
          </div>
        )}
      </Show>
    </>
  );
}

/** No sections page: config.edn's queries, then the agenda (unchanged). */
function ConfigSections(props: { today: string }): JSX.Element {
  return (
    <>
      <For each={graphMeta()?.journal_config_diagnostics ?? []}>
        {(message) => <div class="config-diagnostic" role="status">{t("journal-config-diagnostic", { message })}</div>}
      </For>
      <div class="today-queries">
        <For each={graphMeta()?.default_journal_queries ?? []}>
          {(query) => <div class="default-journal-query">
            <Show when={!query.error} fallback={<div class="query-error" role="alert">{t("journal-query-error", { title: query.title ?? t("journal-query-untitled"), error: query.error ?? "" })}</div>}>
              <QueryMacro body={query.body} title={query.title ?? undefined} currentPage={props.today} hideWhenEmpty configQuery />
            </Show>
          </div>}
        </For>
      </div>
      <Show when={!graphMeta()?.disable_scheduled_and_deadline_query}>
        <div class="agenda-block">
          <QueryMacro body={agendaQuery()} title={t("journal-agenda-title")} builtIn hideWhenEmpty />
        </div>
      </Show>
    </>
  );
}

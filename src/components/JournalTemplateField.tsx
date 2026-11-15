import { For, Show, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { setJournalTemplate, graphMeta } from "../graphSession";
import { graphOwner, readOwned } from "../owned";
import { readOr } from "../resourceRead";
import { reportUiFailure } from "../uiFailure";
import { closeSettings } from "../ui";
import { openPage } from "../router";
import { t } from "../i18n";
import { Rich } from "../i18n/Rich";
import { settings } from "./settingDefinitions";
import { Field, settingHint } from "./settingsField";
import { ResourceFailure } from "./ResourceFailure";
import { createGraphResource } from "../graphResource";

/** Journal template names with on-demand navigation to the selected definition. */
export function JournalTemplateField(): JSX.Element {
  let alive = true; onCleanup(() => { alive = false; });
  const [templatesResource, { refetch }] = createGraphResource(() => true, async () => {
    const owner = graphOwner(() => alive);
    try { const result = await readOwned(owner, backend().listTemplateNames()); return result.kind === "current" ? result.value : undefined; }
    catch (error) { if (owner()) reportUiFailure("template-read", error); throw error; }
  });
  const current = () => graphMeta()?.default_journal_template ?? "";
  const list = () => readOr(templatesResource, undefined, "journal templates") ?? [];
  const selected = () => list().includes(current());
  const missing = () => templatesResource.error === undefined && !templatesResource.loading && current() !== "" && !selected();
  const edit = async () => {
    const name = current();
    const owner = graphOwner(() => alive && current() === name);
    try {
      const result = await readOwned(owner, backend().listTemplates(name));
      if (result.kind === "stale") return;
      const template = result.value[0];
      if (template) { openPage(template.page, template.kind); closeSettings(); }
      // The definition left the graph after the list was read: show it missing.
      else void refetch();
    } catch (error) { if (owner()) reportUiFailure("template-read", error); }
  };
  return (
    <Field
      setting={settings["settings-ui-new-journal-template"]}
      hint={
        <><Rich message={settingHint(settings["settings-ui-new-journal-template"])} /></>
      }
    >
      <div class="settings-jtmpl">
        <ResourceFailure of={templatesResource} what="journal templates" onRetry={() => void refetch()} />
        <select
          class="settings-select"
          value={current()}
          disabled={templatesResource.error !== undefined || templatesResource.loading}
          onChange={(e) => setJournalTemplate(e.currentTarget.value || null)}
        >
          <option value="">{t("settings-ui-none-blank-days")}</option>
          <Show when={missing() || templatesResource.error !== undefined}>
            <option value={current()}>{t("settings-journal-template-missing", { name: current(), state: templatesResource.error === undefined ? "missing" : "unavailable" })}</option>
          </Show>
          <For each={list()}>{(name) => <option value={name}>{name}</option>}</For>
        </select>
        <Show when={selected()}>
          <button class="settings-link" onClick={() => void edit()}>{t("settings-ui-edit")}</button>
        </Show>
      </div>
    </Field>
  );
}

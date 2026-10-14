import { msg, t } from "../i18n";
import { For, Show, createResource, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { pinyinMatchingEnabled } from "../launcherRanking";
import { graphMeta, setGraphMeta } from "../graphSession";
import { bindingOwner, graphOwner, readOwned, writeOwned } from "../owned";
import { pushToast } from "../toasts";
import { readOr } from "../resourceRead";

/** Settings picker for an existing ordinary page as graph home. Search is
 * bounded to eight graph pages; one selected name is written through the
 * config transaction. Failed writes keep the old value and show an error. */
export function HomePageSetting(): JSX.Element {
  const [query, setQuery] = createSignal("");
  const [picking, setPicking] = createSignal(false);
  const [matchesResource] = createResource(query, async (text) => {
    const owner = graphOwner();
    if (!owner()) return [];
    const result = await readOwned(owner, backend().quickSwitch(text, 8, undefined, pinyinMatchingEnabled()));
    return result.kind === "current" ? result.value.filter((page) => page.kind === "page") : [];
  });
  const matches = () => readOr(matchesResource, undefined, "home page search");
  const commit = async (name: string | null) => {
    const owner = bindingOwner();
    const previous = graphMeta();
    if (!owner() || !previous) return;
    try {
      const result = await writeOwned(owner, backend().setDefaultHome(name));
      if (result.kind === "stale") return;
      if (graphMeta()?.root !== previous.root) return;
      setGraphMeta({ ...graphMeta()!, default_home: name });
      setPicking(false);
      setQuery("");
    } catch (error) {
      if (owner()) pushToast(msg("home-page-setting-could-not-save-home-page", { error: String(error) }), "error");
    }
  };
  return (
    <div class="settings-field" data-setting-label="Home page">
      <div class="settings-field-row">
        <span class="settings-label">{t("home-page-setting-home-page")}</span>
        <div class="settings-field-control">
          <Show when={graphMeta()?.default_home && !picking()} fallback={
            <div>
              <input class="settings-input" aria-label={t("home-page-setting-search-home-pages")} placeholder={t("home-page-setting-search-pages")}
                value={query()} onInput={(event) => setQuery(event.currentTarget.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && matches()?.[0]) void commit(matches()![0].name);
                  if (event.key === "Escape") { setPicking(false); setQuery(""); }
                }} />
              <For each={matches() ?? []}>{(page) =>
                <button class="settings-btn" onClick={() => void commit(page.name)}>{page.name}</button>
              }</For>
              <Show when={picking()}><button class="settings-btn" onClick={() => setPicking(false)}>{t("home-page-setting-cancel")}</button></Show>
            </div>
          }>
            <span class="settings-value" data-home-page-value>{graphMeta()?.default_home}</span>
            <button class="settings-btn" onClick={() => setPicking(true)}>{t("home-page-setting-change")}</button>
            <button class="settings-btn" onClick={() => void commit(null)}>{t("home-page-setting-clear")}</button>
          </Show>
        </div>
      </div>
      <div class="settings-hint settings-field-hint">{t("home-page-setting-open-this-page-when-the-graph-opens-only")}</div>
    </div>
  );
}

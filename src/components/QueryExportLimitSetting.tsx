import { t } from "../i18n";
import { onMount, type JSX } from "solid-js";
import { queryExportBudgetMiB, changeQueryExportBudgetMiB, initQueryExportBudget } from "../queryExportBudget";

export function QueryExportLimitSetting(): JSX.Element {
  onMount(() => void initQueryExportBudget());
  return <div class="settings-field" data-setting-label="Query export size limit">
    <div class="settings-field-row"><span class="settings-label">{t("query-export-limit-setting-query-export-size-limit")}</span>
      <div class="settings-field-control"><input class="settings-input" type="number" min="1" max="1048576"
        aria-label={t("query-export-limit-setting-query-export-size-limit-mi-b")} placeholder={t("query-export-limit-setting-default-1-gi-b")} value={queryExportBudgetMiB() ?? ""}
        onChange={(e) => changeQueryExportBudgetMiB(e.currentTarget.value.trim() ? Number(e.currentTarget.value) : null)} />{t("query-export-limit-setting-mi-b")}<button class="settings-btn" onClick={() => changeQueryExportBudgetMiB(null)}>{t("query-export-limit-setting-reset")}</button>
      </div></div>
    <div class="settings-hint">{t("query-export-limit-setting-default-1-gi-b-copied-assets-must-fit")}</div>
  </div>;
}

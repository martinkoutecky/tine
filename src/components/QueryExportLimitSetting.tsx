import { onMount, type JSX } from "solid-js";
import { queryExportBudgetMiB, changeQueryExportBudgetMiB, initQueryExportBudget } from "../queryExportBudget";

export function QueryExportLimitSetting(): JSX.Element {
  onMount(() => void initQueryExportBudget());
  return <div class="settings-field" data-setting-label="Query export size limit">
    <div class="settings-field-row"><span class="settings-label">Query export size limit</span>
      <div class="settings-field-control"><input class="settings-input" type="number" min="1" max="1048576"
        aria-label="Query export size limit (MiB)" placeholder="Default: 1 GiB" value={queryExportBudgetMiB() ?? ""}
        onChange={(e) => changeQueryExportBudgetMiB(e.currentTarget.value.trim() ? Number(e.currentTarget.value) : null)} /> MiB
        <button class="settings-btn" onClick={() => changeQueryExportBudgetMiB(null)}>Reset</button>
      </div></div>
    <div class="settings-hint">Default: 1 GiB. Copied assets must fit this limit; an oversized export stops. Saved on this device.</div>
  </div>;
}

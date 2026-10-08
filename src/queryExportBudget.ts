/** Device-local asset allowance. A null override delegates the 1 GiB default
 * to Rust's query publisher, the single answerer for that default. Preferences
 * use the shared revision/queued-write door; failed writes roll back and toast. */
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { preferenceRevision, preferenceReadCurrent, seedPreference, writePreference } from "./preferenceWrites";
import { readOwned, revisionOwner } from "./owned";
import { pushToast } from "./toasts";

const KEY = "query_export_asset_budget_mib";
const [budget, applyBudget] = createSignal<number | null>(null);
export const queryExportBudgetMiB = budget;
export const queryExportBudgetBytes = () => budget() === null ? undefined : budget()! * 1024 * 1024;
function normalize(value: number): number { return Math.min(1024 * 1024, Math.max(1, Math.round(value))); }
export function changeQueryExportBudgetMiB(value: number | null): void {
  if (value !== null && !Number.isFinite(value)) return;
  const next = value === null ? null : normalize(value);
  writePreference(budget, applyBudget, next, (n) => backend().setAppString(KEY, n === null ? "" : String(n)), "query export size limit");
}
let loaded: Promise<void> | undefined;
/** Load once before using the setting; a concurrent preference write wins. */
export function initQueryExportBudget(): Promise<void> {
  return loaded ??= (async () => {
    const revision = preferenceRevision(budget);
    try {
      const result = await readOwned(revisionOwner(budget, revision, () => preferenceReadCurrent(budget, revision)), backend().getAppString(KEY, ""));
      if (result.kind === "current") {
        const stored = result.value;
        const parsed = Number(stored);
        applyBudget(stored.trim() && Number.isFinite(parsed) ? normalize(parsed) : null);
        seedPreference(budget);
      }
    } catch { if (preferenceReadCurrent(budget, revision)) pushToast("Could not load query export size limit.", "error"); }
  })();
}

import { Show, onMount, type JSX } from "solid-js";
import { t } from "../i18n";
import { ensureSeenAvailability, forgetSeenStateNow, markGraphSeenNow, seenAvailable } from "../seen/seenState";
import { Field } from "./settingsField";

/** Settings → Graph → Changed since you last looked (ADR 0079): the whole-graph mark and the device-local forget.
 *  Shown on every platform where the backend can read the graph's git history. */
export function SeenSettings(): JSX.Element {
  onMount(() => void ensureSeenAvailability());
  return <Show when={seenAvailable() === true}>
    <Field label={t("seen-settings-label")} hint={t("seen-settings-hint")}>
      <button type="button" class="settings-btn" data-seen-mark-graph onClick={() => void markGraphSeenNow()}>
        {t("seen-settings-mark-graph")}
      </button>
      <button type="button" class="settings-btn" data-seen-forget onClick={() => void forgetSeenStateNow()}>
        {t("seen-settings-forget")}
      </button>
    </Field>
  </Show>;
}

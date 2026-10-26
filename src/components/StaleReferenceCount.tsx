import { Show } from "solid-js";
import { t } from "../i18n";

export function StaleReferenceCount(props: { stale: boolean }) {
  return <Show when={props.stale}><span class="references-count-stale" role="img"
    aria-label={t("references-count-stale")} title={t("references-count-stale")}> ·</span></Show>;
}

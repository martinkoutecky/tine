import { Show } from "solid-js";
import { t } from "../i18n";

/** The count was read before a change that may have moved it (a collapsed
 *  section keeps its last answer). A small refresh glyph with the tooltip, not
 *  a bare " ·" that read as stray punctuation after the count (F8). */
export function StaleReferenceCount(props: { stale: boolean }) {
  return <Show when={props.stale}><span class="references-count-stale" role="img"
    aria-label={t("references-count-stale")} title={t("references-count-stale")}>
    <svg viewBox="0 0 24 24" width="12" height="12" aria-hidden="true" fill="currentColor"><path d="M17.65 6.35A7.96 7.96 0 0 0 12 4a8 8 0 1 0 7.73 10h-2.08A6 6 0 1 1 12 6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z" /></svg>
  </span></Show>;
}

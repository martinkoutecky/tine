import { backend } from "../backend";
import { t } from "../i18n";
import { openPdf } from "../panes";
import type { SpanDomAttrs } from "./spans";

/** Only this host-generated handler can authorize the outside-file source.
 * Raw HTML and plugin decorations cannot attach it by imitating DOM attributes. */
export function ExternalPdfLink(props: { dest: string; path: string; label: string; spanAttrs?: SpanDomAttrs }) {
  return <a class="external-link pdf-link" title={props.path} {...(props.spanAttrs ?? {})} onClick={(e) => {
    e.preventDefault(); e.stopPropagation();
    const api = backend(), generation = api.graphBindingGeneration(), dest = props.dest;
    openPdf(dest, props.label, undefined, undefined, { externalRead: () => api.readExternalPdf
      ? api.readExternalPdf(dest, generation) : Promise.reject(new Error(t("pdf-external-desktop-only"))) });
  }}>📄 {props.label}</a>;
}

import { backend } from "../backend";
import { withFileConsent } from "../fileConsent";
import { t } from "../i18n";
import { bindingOwner } from "../owned";
import { openPdf } from "../panes";
import type { SpanDomAttrs } from "./spans";

/** Only this host-generated handler can authorize the outside-file source.
 * Raw HTML and plugin decorations cannot attach it by imitating DOM attributes.
 * The read asks first when the PDF is outside the graph (tine#985); the prompt
 * and the read belong to the pending document (`signal`): when its last view
 * closes, the prompt is cancelled and nothing is read. */
export function ExternalPdfLink(props: { dest: string; path: string; label: string; spanAttrs?: SpanDomAttrs }) {
  return <a class="external-link pdf-link" title={props.path} {...(props.spanAttrs ?? {})} onClick={(e) => {
    e.preventDefault(); e.stopPropagation();
    const api = backend(), generation = api.graphBindingGeneration(), dest = props.dest;
    openPdf(dest, props.label, undefined, undefined, { externalRead: async (signal) => {
      if (!api.readExternalPdf) throw new Error(t("pdf-external-desktop-only"));
      const read = api.readExternalPdf.bind(api);
      const result = await withFileConsent(bindingOwner(() => !signal.aborted),
        purpose => read(dest, generation, purpose), { signal });
      if (result.kind === "current") return result.value;
      throw new Error(result.kind === "cancelled" ? "external-pdf-not-opened" : "stale PDF graph binding");
    } });
  }}>📄 {props.label}</a>;
}

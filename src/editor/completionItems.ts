import { backend } from "../backend";
import { changed } from "../changeKeys";
import { graphEpoch } from "../graphSession";
import { graphOwner, readOwned } from "../owned";
import { msg } from "../i18n";
import { pushToast } from "../toasts";
import { prepareTemplateVars } from "./templateVars";
export { templateToOutline } from "./templateVars";

export interface AcItem {
  label: string;
  /** Secondary, dimmer line (e.g. the page a block-ref candidate lives on). */
  sub?: string;
  insert?: string;
  caret?: number;
  action?: import("./autocomplete").CommandAction;
  taskMarker?: string;
  plugin?: { pluginId: string; contributionId: string; insertText?: string };
  templateNodes?: import("../types").BlockDto[];
  /** A `((block reference))` candidate. `uuid` finds the target; `externalId` is persisted. */
  blockRef?: { uuid: string; externalId: string; page: string; kind: import("../types").PageKind };
  /** Canonical property-name candidate, or a newly folded typed key. */
  propertyName?: string;
  /** Existing or newly typed value for the active canonical property. */
  propertyValue?: string;
}


let templateCache: import("../types").TemplateDto[] | null = null;
let templateCacheRev = "";
let templateCacheEpoch = -1;
export async function getTemplates(): Promise<import("../types").TemplateDto[]> {
  // Re-fetch when the graph has changed since the last fetch,
  // so a template just created (here or externally) shows up without a reload.
  const rev = changed("templates"), epoch = graphEpoch();
  if (templateCache && templateCacheRev === rev && templateCacheEpoch === epoch) return templateCache;
  const owner = graphOwner(() => changed("templates") === rev && graphEpoch() === epoch);
  try {
    const templates = await readOwned(owner, backend().listTemplates());
    if (templates.kind === "stale") return []; templateCache = templates.value;
    templateCacheRev = rev;
    templateCacheEpoch = epoch;
    if (templateCache.length) await prepareTemplateVars();
  } catch (error) {
    // I-9: a failed template listing is reported (sticky red toast with Copy, recorded in the error
    // log) and the last good list, if any, is kept; it is NOT replaced by "no templates", so the
    // next `/` retries once the cause is fixed. The slash menu still lists its commands.
    if (owner()) pushToast(msg("editor-block-couldn-t-load-templates", { detail: String(error) }), "error");
  }
  return templateCache ?? [];
}



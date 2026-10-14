import { existingBlockId } from "../blockIdentity";
import { pushToast } from "../toasts";
import { t } from "../i18n";
import type { BlockDto, Format } from "../types";
import { blockRegions, editBlock } from "../render/parse";

/** Runtime IDs are positional locators after reload. Only an authored ID may
 * survive content changes; an ID-less hit must still match its original content.
 * Page/path validation belongs to the caller. No writes or identity stamping. */
export function sameQueryResult(hit: Pick<BlockDto, "id" | "raw">, source: Pick<BlockDto, "id" | "raw">, format: Format): boolean {
  const durable = existingBlockId(hit.raw, format);
  return durable
    ? existingBlockId(source.raw, format) === durable
    : hit.id === source.id && contentFingerprint(hit.raw, format) === contentFingerprint(source.raw, format);
}

/** Folding changes presentation metadata, not the selected content identity.
 * Let lsdoc remove that property; never recognize authored syntax ourselves. */
function contentFingerprint(raw: string, format: Format): string {
  return editBlock(raw, format, { kind: "property", key: "collapsed", value: null }).trimEnd();
}

export function noticeChangedResults(count: number): void {
  if (count) pushToast(t("query-calm-results-changed", { count }), "error");
}

/** Page-property hits are synthetic read-only rows. Validate their parser-owned
 * properties against the current physical header rather than a runtime node. */
export function samePagePropertyResult(hit: BlockDto, header: string | null, format: Format): boolean {
  if (!header) return false;
  const properties = (raw: string) => blockRegions(raw, format).properties
    .filter(property => property.primary).map(property => [property.key.toLowerCase(), property.value]);
  const selected = properties(hit.raw);
  return selected.length > 0 && JSON.stringify(selected) === JSON.stringify(properties(header));
}

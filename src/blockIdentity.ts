import type { BlockDto, Format } from "./types";
import { blockRegions, type BlockIdentityFacts } from "./render/parse";
import { knownIdentityAbsent } from "./render/facets";

/**
 * The durable identity shipped by the backend for a block result. The DTO's
 * `id` remains the live/runtime identity; its authored `id` property is what
 * persisted references and routes must carry.
 */
export function blockDtoExternalId(block: Pick<BlockDto, "id" | "properties">): string {
  for (const [key, value] of block.properties ?? []) {
    if (key.trim().toLowerCase() !== "id") continue;
    const authored = value.trim();
    if (authored) return authored;
  }
  return block.id;
}

/** The block's existing durable `id` — a markdown `id:: <uuid>` trailer or an
 *  org `:PROPERTIES:` drawer `:id: <uuid>` line — case-insensitively, or null.
 *  Format-aware because in ORG `id:: x` is plain body text, NOT a property (lsdoc
 *  reads the drawer, not a `key::` line); so an org block's real id lives in its
 *  `:PROPERTIES:` drawer and must be matched there (GH #25). Optional editor
 *  facts take precedence; mismatched raw/format throws. Exact loaded parser facts
 *  skip known absence; unknown/possible ids keep the parser answer. Cost:
 *  O(block bytes) lookup, with a parse only when regions are not already cached. */
export function existingBlockId(raw: string, format: Format, facts?: BlockIdentityFacts): string | null {
  if (facts) {
    if (facts.raw !== raw || facts.format !== format) throw new Error("Identity facts belong to a different buffer");
    return facts.value?.trim() || null;
  }
  if (knownIdentityAbsent(raw, format)) return null;
  return blockRegions(raw, format).id?.value.trim() || null;
}

const reservedIds = new WeakMap<ReturnType<typeof blockRegions>, readonly string[]>();
/** Conservative collision policy: reserve every parser-accepted id property,
 * including a recognized drawer of the other format or a non-primary drawer.
 * Unlike external identity (existingBlockId), reservation is not publication.
 * Literal id examples are never properties. O(block bytes) cold; warm reads
 * reuse the parser's bounded cache and allocate nothing. */
export function acceptedBlockIdentityClaims(raw: string, format: Format): readonly string[] {
  const regions = blockRegions(raw, format);
  let ids = reservedIds.get(regions);
  if (!ids) {
    ids = regions.properties.filter((property) => property.key.toLowerCase() === "id")
      .map((property) => property.value.trim()).filter(Boolean);
    reservedIds.set(regions, ids);
  }
  return ids;
}

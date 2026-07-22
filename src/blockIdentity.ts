import type { BlockDto } from "./types";

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

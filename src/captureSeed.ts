import type { PageDto } from "./types";

export const CAPTURE_SCRATCH_NAME = "·capture·";

/**
 * Build the isolated one-block page used by Quick Capture. Its root must have a
 * real id: editor activation intentionally treats an empty id as "no block".
 */
export function createCaptureScratchPage(blockId: string = crypto.randomUUID()): PageDto {
  if (!blockId.trim()) throw new Error("Quick Capture scratch block id must not be empty");
  return {
    name: CAPTURE_SCRATCH_NAME,
    kind: "page",
    title: CAPTURE_SCRATCH_NAME,
    pre_block: null,
    blocks: [{ id: blockId, raw: "", collapsed: false, children: [] }],
    rev: null,
  };
}

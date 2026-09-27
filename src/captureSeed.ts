import type { PageDto } from "./types";
import { captureScratchPage } from "./document";

export const CAPTURE_SCRATCH_NAME = "·capture·";

/**
 * Build the isolated one-block page used by Quick Capture. Its root must have a
 * real id: editor activation intentionally treats an empty id as "no block".
 */
export function createCaptureScratchPage(blockId: string = crypto.randomUUID()): PageDto {
  return captureScratchPage(CAPTURE_SCRATCH_NAME, blockId);
}

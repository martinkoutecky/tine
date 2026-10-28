import { pageToDto } from "./convert";
import { doc } from "./model";
import type { BlockDto } from "../types";

/** One block of a seen snapshot: its runtime id and its raw text at snapshot time. */
export interface SeenSnapshotNode {
  id: string;
  raw: string;
  children: SeenSnapshotNode[];
}

/** A loaded page as "changed since you last looked" asks about it (ADR 0079):
 *  the text the page would save now (unsaved edits included) and the runtime
 *  block ids in that text's structural order, so a backend block path maps back
 *  onto the rendered tree. */
export interface SeenSnapshot {
  /** Graph-relative file path, extension included. */
  path: string;
  text: string;
  roots: SeenSnapshotNode[];
}

function nodes(blocks: BlockDto[]): SeenSnapshotNode[] {
  return blocks.map((block) => ({ id: block.id, raw: doc.byId[block.id]?.raw ?? block.raw, children: nodes(block.children) }));
}

function markdown(blocks: BlockDto[], level: number, out: string[]): void {
  const indent = "\t".repeat(level);
  for (const block of blocks) {
    const [first = "", ...rest] = block.raw.split("\n");
    out.push(first === "" ? `${indent}-` : `${indent}- ${first}`);
    for (const line of rest) out.push(line === "" ? "" : `${indent}  ${line}`);
    markdown(block.children, level + 1, out);
  }
}

function org(blocks: BlockDto[], level: number, out: string[]): void {
  const stars = "*".repeat(level);
  for (const block of blocks) {
    const [first = "", ...rest] = block.raw.split("\n");
    out.push(first === "" ? stars : `${stars} ${first}`);
    out.push(...rest);
    org(block.children, level + 1, out);
  }
}

/** The page's seen snapshot, or null when it is not loaded, has no file yet, is
 *  a bundled guide page or holds an invalid header draft. Pure read of the
 *  in-memory document; O(blocks and text bytes of this page). Call it untracked
 *  and only on a read trigger, never per keystroke. */
export function seenPageSnapshot(pageName: string): SeenSnapshot | null {
  const page = doc.pages.find((p) => p.name === pageName);
  if (!page?.id || page.guide) return null;
  const dto = pageToDto(pageName);
  if (!dto) return null;
  const lines: string[] = [];
  if (dto.pre_block) {
    lines.push(...dto.pre_block.split("\n"));
    if (dto.blocks.length && dto.format !== "org") lines.push("");
  }
  (dto.format === "org" ? org : markdown)(dto.blocks, dto.format === "org" ? 1 : 0, lines);
  return { path: page.id, text: `${lines.join("\n")}\n`, roots: nodes(dto.blocks) };
}

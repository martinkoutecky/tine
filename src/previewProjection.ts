import type { BlockDto } from "./types";

/** Browser DTO preview policy: at least one node, no native estimated-byte cap.
 * O(subtree nodes) to count omissions; copies only the admitted preorder prefix.
 * Published snapshots select owned metadata; in-memory mock DTOs can retain
 * their existing borrowed metadata policy without extra cloning.
 * Native parsed documents use tine_core::projection with a byte budget as well.
 */
export function previewDtoSubtree(root: BlockDto, maxNodes: number, metadata: "owned" | "borrowed"): { blocks: BlockDto[]; truncated: number } {
  let emitted = 0;
  let truncated = 0;
  const count = (block: BlockDto): number => 1 + block.children.reduce((n, b) => n + count(b), 0);
  const copy = (blocks: BlockDto[]): BlockDto[] => {
    const out: BlockDto[] = [];
    for (const block of blocks) {
      if (emitted >= Math.max(1, maxNodes)) {
        truncated += count(block);
        continue;
      }
      emitted++;
      const shallow: BlockDto = { ...block, children: [] };
      const dto = metadata === "owned" ? structuredClone(shallow) : shallow;
      dto.children = copy(block.children);
      out.push(dto);
    }
    return out;
  };
  const blocks = copy([root]);
  return { blocks, truncated };
}

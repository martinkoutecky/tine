import { graphMeta } from "./graphSession";

/** OG's level when `:ref/default-open-blocks-level` is absent or not an integer
 *  (`state/get-ref-open-blocks-level`). */
const OG_REF_OPEN_BLOCKS_LEVEL = 2;

/** GH #793: the level to which references and query results open by default.
 *  A block at this level or deeper that has children starts folded; level 1 is
 *  the displayed hit itself. Any integer counts (zero and below fold every hit
 *  that has children), as in OG; the graph's config reaches here already parsed. */
export function refDefaultOpenBlocksLevel(): number {
  const level = graphMeta()?.ref_default_open_blocks_level;
  return typeof level === "number" && Number.isInteger(level) ? level : OG_REF_OPEN_BLOCKS_LEVEL;
}

/** Whether OG's `block-default-collapsed?` folds a result block shown `depth`
 *  levels below its hit (the hit is depth 0, OG level 1): `level >= N` and it
 *  has children. The block's own `collapsed::` is applied by the caller. */
export function foldsByDefaultInResults(depth: number, hasChildren: boolean): boolean {
  return hasChildren && depth + 1 >= refDefaultOpenBlocksLevel();
}

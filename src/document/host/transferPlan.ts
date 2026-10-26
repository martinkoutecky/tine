import type { BlockDto, PageDto } from "../../types";
import type { SequenceStep } from "./planner";

type TransferEdge = readonly [source: string, destination: string];

function idsOf(blocks: readonly BlockDto[], into = new Set<string>()): Set<string> {
  for (const block of blocks) {
    into.add(block.id);
    idsOf(block.children, into);
  }
  return into;
}

/** The maximal subtrees of `blocks` whose root came from `from`. */
function unitsFrom(blocks: readonly BlockDto[], from: Set<string>, into: BlockDto[] = []): BlockDto[] {
  for (const block of blocks) {
    if (from.has(block.id)) into.push(block);
    else unitsFrom(block.children, from, into);
  }
  return into;
}

function without(blocks: readonly BlockDto[], ids: Set<string>): BlockDto[] {
  return blocks.filter((block) => !ids.has(block.id))
    .map((block) => ({ ...block, children: without(block.children, ids) }));
}

/** Step i moves its edge's blocks: every page's text is the edit's result minus
 * the blocks later steps bring in, plus the blocks later steps take out
 * (appended at the end: a transitional state a crash between steps can leave).
 * Null when the edges do not partition the moved blocks (nothing is sent). */
export function planSteps(endpoints: string[], before: ReadonlyMap<string, PageDto>, after: ReadonlyMap<string, PageDto>,
  moves: readonly TransferEdge[]): SequenceStep[] | null {
  const units = moves.map(([source, destination]) => unitsFrom(after.get(destination)!.blocks, idsOf(before.get(source)!.blocks)));
  const textAt = (name: string, done: number): PageDto => {
    const arriving = new Set<string>();
    const leaving: BlockDto[] = [];
    moves.forEach(([source, destination], j) => {
      if (j < done) return;
      if (destination === name) for (const unit of units[j]) arriving.add(unit.id);
      if (source === name) leaving.push(...units[j]);
    });
    const dto = after.get(name)!;
    return { ...dto, blocks: [...without(dto.blocks, arriving), ...leaving] };
  };
  const census = (done: number) => endpoints.flatMap((name) => [...idsOf(textAt(name, done).blocks)]).sort().join("\u0000");
  const whole = census(moves.length);
  for (let done = 0; done < moves.length; done += 1) if (census(done) !== whole) return null;
  return moves.map(([source, destination], i) => ({
    source, receiver: destination, sourceDto: textAt(source, i + 1), receiverDto: textAt(destination, i + 1),
  }));
}


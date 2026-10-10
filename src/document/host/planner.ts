// Planned multi-page sequences (STEP3-DESIGN §8): K sources into one receiver as
// a sequence of model moves. Step 3b P2a: unwired.

import type { EditKinds } from "../../editKind";
import type { PageDto } from "../../types";
import type { Answered, HostClient } from "./client";

export interface SequenceStep { source: string; receiver: string; sourceDto: PageDto; receiverDto: PageDto }

export type SequenceOutcome =
  | { ok: true; completed: number }
  | { ok: false; completed: number; reason: "commit-failed" | "not-drained" | "refused"; pages: string[];
    refusal?: Answered["refusal"] };

/** Freeze the endpoints (awaiting their component commits), open and drain them,
 * plan the steps from the drained texts, and run each step as one move on the
 * versions the previous step's answers left. A refusal stops the sequence: the
 * completed steps stand, each endpoint keeps its latest authoritative state
 * through the ordinary answer rules, and the caller names what did not move. */
export async function runSequence(
  client: HostClient,
  endpoints: readonly string[],
  plan: (drained: ReadonlyMap<string, PageDto>) => SequenceStep[],
  kinds: EditKinds,
): Promise<SequenceOutcome> {
  const frozen = await client.freeze(endpoints);
  if (!frozen.ok) return { ok: false, completed: 0, reason: "commit-failed", pages: frozen.failed };
  const releases = endpoints.map((name) => client.acquire(name));
  try {
    await client.drain(endpoints);
    const stuck = endpoints.filter((name) => !client.isOpen(name) || client.busy(name) || client.conflicted(name));
    if (stuck.length) return { ok: false, completed: 0, reason: "not-drained", pages: stuck };
    const drained = new Map(endpoints.map((name) => [name, client.doc.dto(name)!] as const));
    const steps = plan(drained);
    for (const [i, step] of steps.entries()) {
      const answer = await client.move(step.source, step.receiver, step.sourceDto, step.receiverDto, kinds);
      if (!answer.took)
        return { ok: false, completed: i, reason: "refused", pages: steps.slice(i).map((rest) => rest.source),
          refusal: answer.refusal };
    }
    return { ok: true, completed: steps.length };
  } finally {
    for (const release of releases) release();
    frozen.unfreeze();
  }
}

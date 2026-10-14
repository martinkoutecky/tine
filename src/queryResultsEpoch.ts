import { createSignal } from "solid-js";

/** Frontend invalidation, not a backend revision or persisted identity. */
const [epoch, setResultsEpoch] = createSignal(0);
export const resultsEpoch = epoch;
const [pending, setPending] = createSignal(0);
/** "own-work": the user's own saves and their publication. They delay reruns
 * (no adoption until settled) but cannot replace a result block, so they do not
 * block result actions. "hold" (default): held external changes, conflicts,
 * deferred reloads, transfers; actions wait until the user resolves them. */
type GateKind = "own-work" | "hold";
const gates: { gate: () => boolean; kind: GateKind }[] = [];
let generation = 0;
export function installResultsPendingGate(gate: () => boolean, kind: GateKind = "hold"): void { gates.push({ gate, kind }); }
export function resultsSettled(): boolean { return pending() === 0 && !gates.some(({ gate }) => gate()); }
export function advanceResultsEpoch(): void { setResultsEpoch(value => value + 1); }

/** Arrival AND completion invalidate: a read during hydration is unusable. */
export function beginResultsChange(): () => void {
  const owner = generation;
  setPending(value => value + 1);
  advanceResultsEpoch();
  let done = false;
  return () => {
    if (done || owner !== generation) return;
    done = true;
    advanceResultsEpoch();
    setPending(value => value - 1);
  };
}
/** Called by the existing document binding lifecycle, without making this
 * low-level signal module import the graph/backend/document cycle. */
export function resetResultsEpoch(): void { generation++; advanceResultsEpoch(); setPending(0); }

export interface ResultsTicket { epoch: number; settled: boolean }
export function captureResultsTicket(): ResultsTicket { return { epoch: resultsEpoch(), settled: resultsSettled() }; }
export function resultsTicketCurrent(ticket: ResultsTicket | undefined): boolean {
  return !!ticket?.settled && ticket.epoch === resultsEpoch() && resultsSettled();
}
/** Acting on an adopted answer needs only that no result change arrived since
 * (every replacement advances the epoch on arrival) and nothing is held for
 * the user to resolve. Requiring settled here refused every result action for
 * ~1 s after the user's own save. */
export function resultsTicketActionable(ticket: ResultsTicket | undefined): boolean {
  return !!ticket?.settled && ticket.epoch === resultsEpoch() && resultsActionable();
}
/** No result change in flight and nothing held for the user; own save work allowed. */
export function resultsActionable(): boolean {
  return pending() === 0 && !gates.some(({ gate, kind }) => kind === "hold" && gate());
}
const provenance = new WeakMap<object, ResultsTicket>();
export function resultTicket(result: object): ResultsTicket | undefined { return provenance.get(result); }

/** Stamp only adopted frontend answers, recursively including result subtrees.
 * Weak ownership keeps IPC DTOs and persisted query syntax unchanged. */
export function adoptResults<T>(value: T, ticket: ResultsTicket): T | undefined {
  if (!resultsTicketCurrent(ticket)) return undefined;
  const visit = (item: unknown) => {
    if (!item || typeof item !== "object") return;
    if (provenance.get(item)?.epoch === ticket.epoch) return;
    provenance.set(item, ticket);
    for (const child of Object.values(item)) visit(child);
  };
  visit(value);
  return value;
}

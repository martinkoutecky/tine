// The one barrier capture (plan v3 §2, REVIEW-3b-plan2 S1): drain the scoped
// work, wait for the versions the host took to publish, and succeed only on a
// final synchronous proof that nothing new arrived. Step 3b P2a: unwired.

import type { HostClient } from "./client";
import type { OwedPage, PublishedNeed } from "./protocol";

export interface SettleOptions {
  /** Await tracked asset writes (destructive transitions, print, orphan scan). */
  assets?: boolean;
  /** Fail when a scoped page is conflicted (today's `flushAll`). */
  noConflict?: boolean;
  /** A block id the scoped pages' published bytes must contain (block refs, Q2). */
  witness?: string;
  /** Today's `flushAll` bound (E96). */
  rounds?: number;
}

export type SettleScope = readonly string[] | "all";

/** True only when the scope's local work is drained, every version the host took
 * from it (and every debt the host owes in it) has published, the asset writes
 * it waits for have finished, and none of that changed across the barrier.
 * Reaching the round bound with new work is false; an empty scope is true. */
export async function settle(client: HostClient, scope: SettleScope, options: SettleOptions = {}): Promise<boolean> {
  for (let round = 0; round < (options.rounds ?? 4); round += 1) {
    if (options.assets) await Promise.all(client.assets.pending());
    // The work identity is taken after the asset wait, so references those
    // writes added are drained in this round; later work changes it.
    const names = scope === "all" ? client.names() : scope;
    const start = { clock: client.editClock, edits: client.editSeqs(names), assets: client.assets.started() };
    await client.drain(names);
    const paths = scope === "all" ? null : pathsOf(client, names);
    // What the host took from this window, read before the debt query: a close
    // answered meanwhile drops the client page, and the host's debt list then
    // names it instead.
    const taken = client.needs(names);
    const owed = await client.owed(paths);
    if (!owed) return false;
    const needs = merge(taken, owed, options.witness);
    if (needs.length) {
      await client.saveNow(needs.map((need) => need.key));
      if (!await client.published(needs)) return false;
    }
    const after = await client.owed(paths);
    if (!after) return false;
    // Final synchronous proof: nothing below awaits, so no edit, answer, asset
    // write or host debt can arrive between this check and the caller's next step.
    if (quiescent(client, scope, start, needs, after, options)) return true;
  }
  return false;
}

function pathsOf(client: HostClient, names: readonly string[]): string[] {
  return names.flatMap((name) => {
    const path = client.doc.facts(name)?.path;
    return path ? [path] : [];
  });
}

function merge(local: { key: string; version: number }[], owed: OwedPage[], witness?: string): PublishedNeed[] {
  const needs = new Map<string, PublishedNeed>();
  for (const { key, version } of [...local, ...owed]) {
    const known = needs.get(key);
    if (!known || known.version < version) needs.set(key, witness ? { key, version, witness } : { key, version });
  }
  return [...needs.values()];
}

function quiescent(
  client: HostClient,
  scope: SettleScope,
  start: { clock: number; edits: number[]; assets: number },
  needs: PublishedNeed[],
  after: OwedPage[],
  options: SettleOptions,
): boolean {
  const names = scope === "all" ? client.names() : scope;
  const proven = new Map(needs.map((need) => [need.key, need.version]));
  const covered = (key: string, version: number) => (proven.get(key) ?? -1) >= version;
  return (scope === "all" ? client.editClock === start.clock
    : client.editSeqs(names).every((seq, i) => seq === start.edits[i]))
    && names.every((name) => !client.busy(name) && !(options.noConflict && client.conflicted(name)))
    && client.needs(names).every((need) => covered(need.key, need.version))
    && after.every((owed) => covered(owed.key, owed.version))
    && (!options.assets || (client.assets.pending().length === 0 && client.assets.started() === start.assets));
}

/** Rename's drain (S6): attempt the All drain, then report every page whose
 * input is not published, whatever the barrier returned, so the caller's
 * selective mentions check always runs. `name` is null for a page only the host
 * holds (a recovered draft); the caller reads its text through the host. */
export async function unpublishedAfterDrain(client: HostClient):
  Promise<{ key: string | null; name: string | null; state: "unsent" | "owed"; conflict: boolean }[]> {
  await settle(client, "all", { assets: true });
  const owed = await client.owed(null) ?? [];
  const local = client.names().filter((name) => client.busy(name));
  const listed = new Set(local);
  return [
    ...local.map((name) => ({ key: client.keyOf(name), name, state: "unsent" as const, conflict: client.conflicted(name) })),
    ...owed.flatMap((entry) => {
      const name = client.nameOfKey(entry.key);
      if (name && listed.has(name)) return [];
      return [{ key: entry.key, name, state: "owed" as const, conflict: !!name && client.conflicted(name) }];
    }),
  ];
}

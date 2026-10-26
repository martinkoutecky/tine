/** Transport-only invalidation. Staging never starts a read: own publications
 * drain at the existing post-publication/refresh-hold bump. Summary-only events
 * in a window without local publication debt use the same trailing/idle gates. */
import { batch, createSignal, untrack } from "solid-js";
import { ChangeKeyRegistry } from "./changeKeyRegistry";
import { clearOnBindingInvalidated } from "./binding";
import type { ChangeSummary } from "./backendTypes";
import { graphEpoch } from "./graphSession";
import { pageIdentityKey } from "./pageIdentity";
import { refreshReadsIdle } from "./refreshReads";
import { installDataRevisionRefresh } from "./dataRevision";

export const [refreshEverything, setRefreshEverything] = createSignal(false);
const [fullRevision, setFullRevision] = createSignal(0);
const keys = new ChangeKeyRegistry();
export const changeKeyCount = () => keys.size;
const invalidators = new Set<(affected: ReadonlySet<string> | null) => void>();
export const changeKeyHasConsumers = (key: string) => keys.hasConsumers(key);
export const onChangeKeyReleased = (callback: () => void) => keys.onRelease(callback);
export const onChangeKeysFlushed = (callback: (affected: ReadonlySet<string> | null) => void) => { invalidators.add(callback); };
let epoch = -1;
let pending = new Set<string>();
let pendingFull = false;
let staged = false;
let timer: ReturnType<typeof setTimeout> | undefined;
let serial = 0;
let publicationHeld = () => false;

/** Installed by the host, without coupling summary transport to host state. */
export function installChangeKeyPublicationHold(held: () => boolean): void { publicationHeld = held; }
export const pageChangeKey = (name: string) => `page:${pageIdentityKey(name)}`;
export const iconChangeKey = (name: string) => `icon:${pageIdentityKey(name)}`;

function reset(): void {
  if (timer) clearTimeout(timer);
  timer = undefined;
  serial++;
  staged = pendingFull = false;
}
clearOnBindingInvalidated(() => {
  epoch = -1; reset(); keys.clear(); pending.clear(); setFullRevision(n => n + 1);
});
function ensureEpoch(): number {
  const next = graphEpoch();
  if (epoch !== next) { reset(); keys.clear(); pending.clear(); epoch = next; }
  return next;
}

/** Reactive revision for exactly one dependency. Unknown key kinds receive full
 * invalidations only. Reactive consumers own keys; imperative history is bounded. */
export function changed(key: string): string {
  const current = ensureEpoch();
  const { instance, revision } = keys.read(key);
  return `${current}:${fullRevision()}:${instance}:${revision}`;
}

/** One token for a dynamic set of dependencies. Adding a freshly shown member
 * at revision zero establishes its subscription without causing another read. */
export function changedAny(dependencies: readonly string[]): string {
  const current = ensureEpoch();
  let revision = 0;
  for (const key of dependencies) {
    revision += keys.read(key).revision;
  }
  return `${current}:${fullRevision()}:${revision}`;
}

export function stageChangeKeys(change: ChangeSummary): void {
  ensureEpoch();
  staged = true;
  // Broad consumers (All Pages metadata and lifecycle wakeups) explicitly
  // depend on publication; this does not turn precise readers into full ones.
  pending.add("publication");
  for (const key of ["days", "journals", "templates"] as const) {
    if (change[key] !== false) pending.add(key);
  }
  pendingFull ||= change.full || untrack(refreshEverything);
  for (const name of change.pages) pending.add(`page:${name}`);
  for (const id of change.blocks) pending.add(`block:${id}`);
  for (const name of change.linksTo) pending.add(`links:${name}`);
  for (const name of change.textTo ?? []) pending.add(`text:${name}`);
  for (const name of change.tagsTo ?? []) pending.add(`tags:${name}`);
  for (const name of change.tagTableTo ?? []) pending.add(`tagTable:${name}`);
  for (const id of change.refsTo ?? []) pending.add(`refsTo:${id}`);
  for (const id of change.queryTo ?? []) pending.add(`query:${id}`);
  // Legacy hosts lack S2 evidence: retain conservative refreshes for just
  // these reader families rather than silently keeping their answers stale.
  for (const [field, prefix] of [[change.textTo, "text:"], [change.tagsTo, "tags:"], [change.tagTableTo, "tagTable:"], [change.refsTo, "refsTo:"], [change.queryTo, "query:"]] as const) {
    if (!field) for (const key of keys.keys()) if (key.startsWith(prefix)) pending.add(key);
  }
  for (const [name, props] of Object.entries(change.props)) {
    for (const prop of props) pending.add(`prop:${prop}`);
    if (props.some(prop => prop === "icon" || prop === "tine.icon")) pending.add(`icon:${name}`);
  }
  if (timer) clearTimeout(timer);
  const ticket = ++serial;
  timer = setTimeout(() => {
    timer = undefined;
    // The host owns the publication wait and its bounded refresh hold.
    if (publicationHeld()) return;
    let done = false;
    const release = () => {
      if (done) return;
      done = true;
      clearTimeout(hold);
      if (ticket === serial && !publicationHeld()) flushChangeKeys(true);
    };
    const hold = setTimeout(release, 5000);
    void refreshReadsIdle().then(release);
  }, 700);
}

/** A caller without summary evidence must fire every key. The save/external
 * callers opt into staged evidence; an absent summary still falls back to full. */
export function flushChangeKeys(hasSummary = false): boolean {
  ensureEpoch();
  const full = !hasSummary || !staged || pendingFull;
  const affected = pending;
  pending = new Set();
  staged = pendingFull = false;
  if (timer) clearTimeout(timer);
  timer = undefined;
  serial++;
  batch(() => {
    for (const invalidate of invalidators) invalidate(full ? null : affected);
    if (full) setFullRevision(n => n + 1);
    else for (const key of affected) keys.advance(key);
  });
  return full;
}
installDataRevisionRefresh(flushChangeKeys);

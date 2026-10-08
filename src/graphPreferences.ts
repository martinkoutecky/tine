/** Graph preferences. Writes apply immediately and serialize per key and graph
 * binding. A graph switch rejects queued writes with a toast; an in-flight
 * persist may finish. Failed active writes restore the last confirmed value.
 * Seeding never writes config. O(1) frontend work plus backend writes. */
import { backend } from "./backend";
import { graphMeta, setGraphMeta } from "./graphSession";
import { writePreference, seedPreference } from "./preferenceWrites";
import { pushToast } from "./toasts";
import type { GraphMeta } from "./types";

const readers = new Map<string, () => unknown>();
let scope = "";

function currentScope(): string {
  return `${graphMeta()?.root ?? ""}\0${backend().graphBindingGeneration?.() ?? 0}`;
}

/** Seed an already tracked key in the current graph scope. No config write;
 * unknown keys and stale scopes do nothing. O(1). */
export function seedGraphSignal(key: string): void {
  if (currentScope() === scope) {
    const read = readers.get(key);
    if (read) seedPreference(read);
  }
}

/** Apply now and queue a graph-bound write. A graph switch rejects a queued
 * write with a toast but cannot cancel one in flight; active failures roll back.
 * Return does not confirm persistence. O(1) plus queued backend work. */
export function writeGraphSignal<T>(
  key: string, read: () => T, apply: (value: T) => void, value: T,
  persist: (value: T) => Promise<unknown>, label: string,
): void {
  const activeScope = currentScope();
  if (activeScope !== scope) { readers.clear(); scope = activeScope; }
  let scopedRead = readers.get(key) as (() => T) | undefined;
  if (!scopedRead) { scopedRead = () => read(); readers.set(key, scopedRead); }
  const bound = () => currentScope() === activeScope;
  writePreference(scopedRead, (next) => { if (bound()) apply(next); }, value,
    (next) => bound()
      ? persist(next)
      : Promise.reject(new Error("graph changed before preference write")),
    label);
}

/** Change graph metadata; equal values do nothing. Without loaded metadata,
 * persist directly without a signal update or queue. Failures toast; return
 * does not confirm persistence. O(1) plus any backend write. */
export function changeGraphSetting<K extends keyof GraphMeta>(
  key: K, value: GraphMeta[K], persist: (value: GraphMeta[K]) => Promise<unknown>, label: string,
): void {
  const meta = graphMeta();
  if (!meta) {
    void persist(value).catch(() => pushToast(`Could not save ${label}.`, "error"));
    return;
  }
  if (meta[key] === value) return;
  writeGraphSignal(`meta:${String(key)}`, () => graphMeta()?.[key] as GraphMeta[K], (next) => {
    const current = graphMeta();
    if (current) setGraphMeta({ ...current, [key]: next });
  }, value, persist, label);
}

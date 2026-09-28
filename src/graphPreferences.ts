/** Graph config preferences. `changeGraphSetting` and `writeGraphSignal` apply
 * one value for the current graph; `seedGraphSignal` records a completed load.
 * Each call does O(1) frontend work plus one serialized config write. Failure
 * restores the last confirmed value and shows an error; a graph switch cancels
 * a queued write. Callers need no queue or graph-binding state. */
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

export function seedGraphSignal(key: string): void {
  if (currentScope() === scope) {
    const read = readers.get(key);
    if (read) seedPreference(read);
  }
}

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

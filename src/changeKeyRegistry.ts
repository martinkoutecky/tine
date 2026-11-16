import { createSignal, getListener, onCleanup, type Signal } from "solid-js";

interface Dependency { signal: Signal<number>; consumers: number; instance: number }
/** Reactive dependencies own their signals. Imperative readers keep only a small
 * history; recreating a discarded key gets a new token, never an old cache hit. */
export class ChangeKeyRegistry {
  private entries = new Map<string, Dependency>();
  private inactive = new Set<string>();
  private sequence = 0;
  private nextInstance = 0;
  private released = new Set<() => void>();
  get size(): number { return this.entries.size; }
  keys(): IterableIterator<string> { return this.entries.keys(); }
  hasConsumers(key: string): boolean { return (this.entries.get(key)?.consumers ?? 0) > 0; }
  onRelease(callback: () => void): void { this.released.add(callback); }

  read(key: string): { instance: number; revision: number } {
    let entry = this.entries.get(key);
    if (!entry) {
      entry = { signal: createSignal(0), consumers: 0, instance: ++this.nextInstance };
      this.entries.set(key, entry);
    }
    if (getListener()) {
      const held = entry;
      held.consumers++;
      this.inactive.delete(key);
      onCleanup(() => {
        if (--held.consumers !== 0) return;
        // Recomputing a resource releases and immediately reacquires its keys.
        // Preserve that signal/revision; dispose an actually abandoned key at
        // the end of this turn, without changing unrelated dependency tokens.
        queueMicrotask(() => {
          if (held.consumers || this.entries.get(key) !== held || this.inactive.has(key)) return;
          this.entries.delete(key);
          for (const callback of this.released) callback();
        });
      });
    } else if (!entry.consumers) {
      this.inactive.delete(key);
      this.inactive.add(key);
      while (this.inactive.size > 512) {
        const oldest = this.inactive.values().next().value!;
        this.inactive.delete(oldest);
        this.entries.delete(oldest);
      }
    }
    return { instance: entry.instance, revision: entry.signal[0]() };
  }

  advance(key: string): void { this.entries.get(key)?.signal[1](++this.sequence); }
  clear(): void { this.entries.clear(); this.inactive.clear(); }
}

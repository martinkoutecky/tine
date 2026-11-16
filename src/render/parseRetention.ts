import type { Block } from "./ast";

/** Historical strong retention plus weak reuse of ASTs owned by current callers. */
export class ParseRetention {
  private entries = new Map<string, { blocks: Block[]; bytes: number }>();
  private recent = new Map<number, WeakRef<Block[]>>();
  private metadata = new WeakMap<Block[], string>();
  private bytes = 0;
  readonly maxBytes = 8 * 1024 * 1024;
  readonly maxEntryBytes = 1024 * 1024;

  get(key: string): Block[] | undefined {
    const hit = this.entries.get(key);
    if (hit) {
      this.entries.delete(key);
      this.entries.set(key, hit);
      return hit.blocks;
    }
    const blocks = this.recent.get(this.hash(key))?.deref();
    if (blocks && this.metadata.get(blocks) === key) return blocks;
  }

  remember(key: string, blocks: Block[], bundleChars: number): Block[] {
    this.metadata.set(blocks, key);
    const hash = this.hash(key);
    this.recent.delete(hash);
    this.recent.set(hash, new WeakRef(blocks));
    if (this.recent.size > 8000) this.recent.delete(this.recent.keys().next().value!);
    // Source strings are UTF-16; JSON includes AST, regions and macro metadata.
    // 8x serialized length conservatively charges strings, objects and arrays.
    const bytes = key.length * 2 + bundleChars * 8 + 256;
    if (bytes > this.maxEntryBytes) return blocks;
    while (this.entries.size >= 8000 || this.bytes + bytes > this.maxBytes) {
      const oldest = this.entries.keys().next().value!;
      this.bytes -= this.entries.get(oldest)!.bytes;
      this.entries.delete(oldest);
    }
    this.entries.set(key, { blocks, bytes });
    this.bytes += bytes;
    return blocks;
  }

  clear(): void {
    this.entries.clear();
    this.recent.clear();
    this.metadata = new WeakMap();
    this.bytes = 0;
  }
  stats(): { entries: number; bytes: number } { return { entries: this.entries.size, bytes: this.bytes }; }

  private hash(key: string): number {
    let hash = 2166136261;
    for (let i = 0; i < key.length; i++) hash = Math.imul(hash ^ key.charCodeAt(i), 16777619);
    return hash >>> 0;
  }
}

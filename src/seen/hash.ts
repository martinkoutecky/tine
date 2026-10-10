// The ONE definition of a block's seen hash (vision decision 9a, ADR 0073). The
// backend stores these hashes as opaque 64-bit values and never computes one,
// so there is no Rust twin of this rule.
import { blockContentKey, childIds, pageRoots } from "../document";
import type { Format } from "../types";

const hex32 = (value: number): string => (value >>> 0).toString(16).padStart(8, "0");

/** 64-bit hash (two 32-bit multiply-xorshift lanes, the cyrb53 construction
 *  kept at full width) of a block's own content: its text with property lines,
 *  never its children, its position or its fold state (`blockContentKey`).
 *  Returned as 16 lowercase hex digits, the form the backend stores. O(raw). */
export function seenBlockHash(raw: string, format: Format): string {
  const text = blockContentKey(raw, format);
  let h1 = 0xdeadbeef ^ text.length;
  let h2 = 0x41c6ce57 ^ text.length;
  for (let i = 0; i < text.length; i++) {
    const unit = text.charCodeAt(i);
    h1 = Math.imul(h1 ^ unit, 2654435761);
    h2 = Math.imul(h2 ^ unit, 1597334677);
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
  return hex32(h2) + hex32(h1);
}

/** Every block id of a loaded page, roots first then depth-first. Reads only
 *  the outline's structure (each node's `children`), so a tracking scope that
 *  calls it re-runs on a structural edit, never on a keystroke. O(page). */
export function pageBlockIds(pageName: string): string[] {
  const out: string[] = [];
  const pending = [...pageRoots(pageName)].reverse();
  while (pending.length) {
    const id = pending.pop()!;
    out.push(id);
    const children = childIds(id);
    for (let i = children.length - 1; i >= 0; i--) pending.push(children[i]);
  }
  return out;
}

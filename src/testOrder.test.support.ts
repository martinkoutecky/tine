import { expect } from "vitest";

/** Asserts that every item occurs in `haystack` and that their first
 *  occurrences come in the given order. Comparing two `indexOf` results alone
 *  passes when the earlier item is missing (-1 is less than any index); this
 *  fails instead. */
export function expectOrderedIn(haystack: string, ...items: string[]): void;
export function expectOrderedIn<T>(haystack: readonly T[], ...items: T[]): void;
export function expectOrderedIn(haystack: string | readonly unknown[], ...items: unknown[]): void {
  if (items.length < 2) throw new Error("expectOrderedIn needs at least two items");
  const label = (item: unknown) => (typeof item === "string" ? JSON.stringify(item) : String(item));
  const positions = items.map((item) => {
    const at = typeof haystack === "string" ? haystack.indexOf(item as string) : haystack.indexOf(item);
    expect(at, `${label(item)} is missing`).toBeGreaterThanOrEqual(0);
    return at;
  });
  for (let i = 1; i < positions.length; i++) {
    expect(positions[i - 1], `${label(items[i - 1])} comes before ${label(items[i])}`).toBeLessThan(positions[i]);
  }
}

// The pages a routed view is showing for "changed since you last looked"
// (ADR 0079): what leaving one needs to call mark_page_seen with. A leaf module
// (no backend import) so backend.ts can drop an entry the moment Tine renames,
// merges or deletes the page: the backend drops that page's record itself, and a
// mark on leave would recreate it under the old key.
import type { DisplayOwner } from "../owned";
import { pageIdentityKey } from "../pageIdentity";

export interface DisplayedSeenPage {
  key: string;
  path: string;
  /** Still the graph binding the page was read under. */
  owner: DisplayOwner;
}

const displayed = new Map<string, DisplayedSeenPage>();

// A route names the page as typed; the page itself carries its file name. Both
// reach the same entry through the page identity key.
function keyOf(name: string): string {
  try { return pageIdentityKey(name); } catch { return name.trim().toLowerCase(); }
}

export function registerDisplayed(name: string, page: DisplayedSeenPage): void {
  displayed.set(keyOf(name), page);
}

/** Remove and return the entry for `name`. */
export function takeDisplayed(name: string): DisplayedSeenPage | undefined {
  const page = displayed.get(keyOf(name));
  displayed.delete(keyOf(name));
  return page;
}

export function clearDisplayed(): void {
  displayed.clear();
}

export function takeAllDisplayed(): DisplayedSeenPage[] {
  const all = [...displayed.values()];
  displayed.clear();
  return all;
}

/** Tine is renaming, merging or deleting this page (by name, or by file path):
 *  it must not be marked seen on leave. */
export function dropDisplayedFor(page: { name?: string; path?: string }): void {
  const wanted = page.name === undefined ? undefined : keyOf(page.name);
  for (const [key, entry] of [...displayed]) {
    if (key === wanted || (page.path !== undefined && entry.path === page.path)) displayed.delete(key);
  }
}

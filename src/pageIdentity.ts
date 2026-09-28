/** Mirror core `refs::page_key`: trim, Unicode lowercase, remove one boundary
 *  slash at each side, then NFC. Lowercasing is contextual (`ΟΣ` → `ος`).
 *  A leaf module so favorites and reference views share the fold without
 *  importing ui.ts. */
export function pageIdentityKey(name: string): string {
  const lowered = name.trim().toLowerCase();
  const withoutLeading = lowered.startsWith("/") ? lowered.slice(1) : lowered;
  const withoutBoundaries = withoutLeading.endsWith("/")
    ? withoutLeading.slice(0, -1)
    : withoutLeading;
  return withoutBoundaries.normalize("NFC");
}

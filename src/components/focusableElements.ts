/** The controls Tab can reach inside a dialog, for its focus trap (shared by the modals that trap focus). */
export function focusableElements(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(
    'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), summary, [href], [tabindex]:not([tabindex="-1"])'
  )].filter((element) => !element.hasAttribute("hidden") && !element.closest("details:not([open])"));
}

/** Keep Tab and Shift+Tab inside `root` (a modal's focus trap). */
export function trapTab(event: KeyboardEvent, root: HTMLElement): void {
  if (event.key !== "Tab") return;
  const focusable = focusableElements(root);
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  if (!first || !last) return;
  const active = root.ownerDocument.activeElement;
  if (event.shiftKey && active === first) { event.preventDefault(); last.focus(); }
  else if (!event.shiftKey && active === last) { event.preventDefault(); first.focus(); }
}

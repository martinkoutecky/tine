/** The controls Tab can reach inside a dialog, for its focus trap (shared by the modals that trap focus). */
export function focusableElements(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(
    'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), summary, [href], [tabindex]:not([tabindex="-1"])'
  )].filter((element) => !element.hasAttribute("hidden") && !element.closest("details:not([open])"));
}

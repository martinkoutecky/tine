// GH #283: ONE modified-click decision for internal page/block links, used by
// every surface (main content, references, query results, search results,
// Favorites, Recent, namespaces), so destinations can only be chosen here —
// not drifted per surface:
//
//   plain click                     → the surface's ordinary navigation;
//   Shift+click                     → right sidebar (Tine's convention);
//   Ctrl(Win/Linux)/Cmd(macOS)+click → background tab;
//   middle-click (per-surface aux)  → background tab;
//   Alt+click                       → the other pane (GH #438).
//
// The Alt destination is the fourth step of GH #288's switcher ladder: Search
// and Quick Switcher rows long opened the other pane on Alt+click/Alt+Enter,
// and ordinary links had no mouse+modifier route to it after #283 gave
// Ctrl/Cmd+click to background tabs. Surfaces stay at the routing boundary —
// they open the pane through `openRouteInOtherPane`, never a layout algorithm
// of their own — and interfaces stay modifier-free: there is no setting for
// this contract (Martin's approved product decision).
//
// Modifier precedence mirrors the switcher's guard chain (GH #288): a PURE
// Shift keeps the sidebar and a PURE Ctrl/Cmd keeps the background tab; Alt
// claims every chord it participates in (Shift+Alt and Ctrl+Alt included), and
// combined modifiers without Alt (shift+ctrl) keep plain-click meaning, so no
// novel chord shadows the established ones.
//
// GH #207 made the two halves around the click decision equally shared:
// `internalLinkMouseDown` (the default-suppression a gesture needs BEFORE the
// click finishes) and `internalLinkAuxClick` (the completed middle-click).
// Surfaces that skipped either half showed browser autoscroll / PRIMARY-paste
// or plain-click-only behavior instead of the contract.
export type InternalLinkDest = "default" | "sidebar" | "background" | "pane";

export function internalLinkDest(
  e: Pick<MouseEvent, "shiftKey" | "ctrlKey" | "metaKey" | "altKey">,
): InternalLinkDest {
  if (e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey) return "sidebar";
  if (!e.shiftKey && (e.ctrlKey || e.metaKey) && !e.altKey) return "background";
  if (e.altKey) return "pane";
  return "default";
}

/** The mousedown half of the contract: suppress the browser defaults that the
 *  destinations replace — shift-range text selection when Shift+click opens
 *  the sidebar (GH #42), and middle-button autoscroll (Windows) /
 *  PRIMARY-paste (Linux) when middle-click opens a background tab. Every
 *  internal-link surface runs its onMouseDown through this so the suppression
 *  cannot drift per surface (GH #207). */
export function internalLinkMouseDown(
  e: Pick<MouseEvent, "shiftKey" | "button" | "preventDefault">,
): void {
  if (e.shiftKey || e.button === 1) e.preventDefault();
}

/** The auxclick half: a completed middle-click resolves to the background-tab
 *  destination on EVERY internal-link surface (GH #207). `openBackground` is
 *  the surface's own tab-opening action. Returns true when the event was
 *  consumed, so surfaces that also stopPropagation can do so conditionally. */
export function internalLinkAuxClick(
  e: Pick<MouseEvent, "button" | "preventDefault">,
  openBackground: () => void,
): boolean {
  if (e.button !== 1) return false;
  e.preventDefault();
  openBackground();
  return true;
}

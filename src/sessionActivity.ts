// When does a recorded session END, for "Tine did not close cleanly"? (GH #426,
// master a846665f; og ADR 0058.) The backend arms a session marker at startup
// and clears it at `RunEvent::Exit`. Android and iOS suspend a hidden app and
// reap it without notice, so on mobile the session follows visibility: it ends
// when the app is hidden and restarts when the user returns — a crash the user
// witnesses is still reported, an OS reap is not. Desktop installs nothing: a
// minimised window is a live session whose crash must still be reported.

export interface SessionActivityDeps {
  /** Tell the backend whether the session should count as live from now on. */
  setActive(active: boolean): void;
  /** Android or iOS. Desktop installs nothing at all. */
  isMobile: boolean;
  isHidden?(): boolean;
  addEventListener?: typeof document.addEventListener;
  removeEventListener?: typeof document.removeEventListener;
}

/** `pagehide`/`freeze` can fire while `visibilityState` still reads "visible",
 *  and mobile WebViews are inconsistent about which of the three they deliver,
 *  so each event carries its own verdict instead of re-reading the document. */
const HIDE_EVENTS = ["pagehide", "freeze"] as const;
const SHOW_EVENTS = ["pageshow", "resume"] as const;

export function installSessionActivity(deps: SessionActivityDeps): () => void {
  if (!deps.isMobile) return () => {};
  const add = deps.addEventListener ?? document.addEventListener.bind(document);
  const remove = deps.removeEventListener ?? document.removeEventListener.bind(document);
  const isHidden = deps.isHidden
    ?? (() => typeof document === "undefined" || document.visibilityState !== "visible");

  // The backend already armed the marker at startup, and these events fire far
  // more often than the state changes (a single background can deliver
  // `visibilitychange` and `pagehide`). Only edges are worth an IPC round trip.
  let active = true;
  const set = (next: boolean) => {
    if (next === active) return;
    active = next;
    deps.setActive(next);
  };

  const onVisibility = () => set(!isHidden());
  const onHide = () => set(false);
  const onShow = () => set(true);

  add("visibilitychange", onVisibility);
  for (const event of HIDE_EVENTS) add(event, onHide);
  for (const event of SHOW_EVENTS) add(event, onShow);
  return () => {
    remove("visibilitychange", onVisibility);
    for (const event of HIDE_EVENTS) remove(event, onHide);
    for (const event of SHOW_EVENTS) remove(event, onShow);
  };
}

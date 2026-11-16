// Leaving a routed page marks it seen (ADR 0079). The trigger is the pane's
// routed page name changing or the pane going away; a page that merely reloads
// (an external change, a re-render) keeps its route and is never marked.
import { createEffect, on, onCleanup, untrack, type Accessor } from "solid-js";
import { leaveSeenPage } from "./seenState";

/** `routed` is the page name the pane's route shows, or null for any other route. */
export function markSeenOnLeave(routed: Accessor<string | null>): void {
  createEffect(on(routed, (current, previous) => {
    if (previous && previous !== current) void leaveSeenPage(previous);
  }));
  onCleanup(() => {
    const current = untrack(routed);
    if (current) void leaveSeenPage(current);
  });
}

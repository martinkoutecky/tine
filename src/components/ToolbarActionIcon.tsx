import { Match, Switch } from "solid-js";
import { JournalsIcon } from "./JournalsIcon";
import { ShareIcon } from "./ShareIcon";
import type { ToolbarActionId } from "../toolbarChoice";

/** Chrome glyphs shared by the toolbar and its customization list. */
export function ToolbarActionIcon(props: { id: ToolbarActionId; light?: boolean }) {
  return <Switch fallback={<svg class="nav-icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.7" /><line x1="15" y1="4" x2="15" y2="20" stroke="currentColor" stroke-width="1.7" /></svg>}>
    <Match when={props.id === "journals"}><JournalsIcon /></Match>
    <Match when={props.id === "share"}><ShareIcon /></Match>
    <Match when={props.id === "calendar"}><svg class="nav-icon" viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="5" width="16" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.7" /><path d="M4 9.5h16M8.5 3v4M15.5 3v4" fill="none" stroke="currentColor" stroke-width="1.7" /></svg></Match>
    <Match when={props.id === "theme" && props.light}><svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true"><circle cx="12" cy="12" r="5" fill="none" stroke="currentColor" stroke-width="1.6" /><path d="M12 2v3M12 19v3M2 12h3M19 12h3" stroke="currentColor" stroke-width="1.6" /></svg></Match>
    <Match when={props.id === "theme"}><svg class="nav-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M21 12.8A9 9 0 1111.2 3 7 7 0 0021 12.8z" fill="none" stroke="currentColor" stroke-width="1.7" /></svg></Match>
    <Match when={props.id === "workspaces"}><span class="workspace-switcher-mark" aria-hidden="true">W</span></Match>
    <Match when={props.id === "settings"}><svg class="nav-icon" viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.488.488 0 00-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 00-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 00-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.07.62-.07.94s.02.64.07.94l-2.03 1.58a.49.49 0 00-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 00-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z" /></svg></Match>
  </Switch>;
}

import { For, Show, type JSX } from "solid-js";
import { conflictReason, conflicts, groupedPages, resolveConflict, waitingFor, waitingOn } from "../document";

// Global save-conflict surface. A save is refused (not clobbered) when the file
// changed on disk under us (external edit / Syncthing). Such a page is parked in
// `conflicts` and skipped by every future save batch until resolved — so it MUST
// be surfaced no matter where the page lives (main view, journals feed, sidebar,
// or a query result), or its edits would be silently stuck and lost on close.
export function ConflictBar(): JSX.Element {
  const waiting = () => [...groupedPages()].filter((name) => !conflicts().includes(name) && waitingFor(name).length);
  return (
    <Show when={conflicts().length > 0 || waiting().length > 0}>
      <div class="conflict-stack">
        <For each={conflicts()}>
          {(name) => {
            const reason = () => conflictReason(name);
            return (
            <div class="conflict-banner">
              <span class="conflict-msg">
                <Show when={reason()?.kind === "released"} fallback={
                  <><strong>“{name}” {reason()?.kind === "repeated" ? "has a repeated file target" : reason()?.kind === "alias-owner-busy" ? "has a busy alias owner" : "changed on disk"}</strong>. Your unsaved changes weren't written.</>
                }>
                  <strong>“{name}” waits for your decision.</strong> You chose the disk version of “{(() => { const r = reason(); return r?.kind === "released" ? r.partner : ""; })()}”. Use disk version here to restore what this page gave to it. Keep mine leaves this page as it is now, so moved content is on no page.
                </Show>
                <Show when={waitingOn(name).length}> Pages waiting on this decision: {waitingOn(name).join(", ")}.</Show>
                <Show when={waitingFor(name).length}> Still needs a decision: {waitingFor(name).join(", ")}.</Show>
              </span>
              <span class="conflict-actions">
                <button class="conflict-btn" onClick={() => void resolveConflict(name, "disk")}>
                  Use disk version
                </button>
                <Show when={reason()?.kind !== "repeated"}>
                  <button class="conflict-btn keep" onClick={() => void resolveConflict(name, "mine")}>
                    Keep mine (overwrite)
                  </button>
                </Show>
              </span>
            </div>
          );}}
        </For>
        <For each={waiting()}>{(name) => <div class="conflict-banner"><span class="conflict-msg">“{name}” is waiting on {waitingFor(name).join(", ")} before its changes can save.</span></div>}</For>
      </div>
    </Show>
  );
}

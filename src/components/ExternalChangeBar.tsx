// Concord P5, the "always ask" surface (master). Renders only while the policy
// holds an external change to this page: a calm bar above the page, never a
// modal. "Reload from disk" re-enters the ordinary external-change handler;
// "Keep mine" drops the record, and the next save meets the base-revision guard.
import { Show, type JSX } from "solid-js";
import { applyHeldExternalChange, dismissHeldExternalChange, heldExternalChangeFor } from "../conflictPolicy";

export function ExternalChangeBar(props: { name: string }): JSX.Element {
  return (
    <Show when={heldExternalChangeFor(props.name)}>
      <div class="external-change-bar" role="status">
        <span class="external-change-text">
          This page changed on disk. You asked to be told rather than shown, so Tine is still
          displaying the version you were reading.
        </span>
        <span class="external-change-actions">
          <button class="settings-btn settings-btn-primary" onClick={() => applyHeldExternalChange(props.name)}>
            Reload from disk
          </button>
          <button class="settings-btn" onClick={() => dismissHeldExternalChange(props.name)}>Keep mine</button>
        </span>
      </div>
    </Show>
  );
}

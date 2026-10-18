import { t } from "../i18n";
import { DiskOrMineChoice } from "./DiskOrMineChoice";
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
        <span class="external-change-text">{t("external-change-bar-this-page-changed-on-disk-you-asked-to")}</span>
        <span class="external-change-actions">
          <DiskOrMineChoice buttonClass="settings-btn"
            onKeepMine={() => dismissHeldExternalChange(props.name)} onUseDisk={() => applyHeldExternalChange(props.name)} />
        </span>
      </div>
    </Show>
  );
}
